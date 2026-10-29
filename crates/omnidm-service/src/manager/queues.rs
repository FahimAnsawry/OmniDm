// SPDX-License-Identifier: GPL-3.0-or-later

//! Queues. A started queue runs its downloads from the top, `max_concurrent` at a time.
//! Every unfinished download in it (waiting, paused or failed) is tried once per run, so
//! one that fails or is paused by hand is passed over instead of retried forever. When
//! nothing is left to try, the run ends with [`Update::QueueFinished`]; stopping a queue
//! pauses its downloads and ends the run without it.

use std::collections::hash_map::Entry;

use jiff::Zoned;
use jiff::civil::{DateTime, Time};
use omnidm_ipc::protocol::{NewQueue, QueueEdit, QueueSchedule, QueueSummary};
use omnidm_queue::schedule::{Days, Start};
use omnidm_store::{
    DownloadId, PostAction, QueueId, QueueRecord, QueueSettings, Schedule, Status, Store,
};
use tokio_util::sync::CancellationToken;

use super::{Manager, QueueRun, Update};

/// The most downloads one queue may run at once.
pub const MAX_QUEUE_CONCURRENT: u32 = 16;
const DEFAULT_QUEUE_CONCURRENT: u32 = 3;
const MAX_NAME_LEN: usize = 64;

impl Manager {
    /// Every queue, Main first.
    pub fn queues(&self) -> Vec<QueueSummary> {
        let stored: Vec<(QueueRecord, Vec<DownloadId>)> = {
            let store = self.inner.store.lock().unwrap();
            store
                .list_queues()
                .unwrap_or_default()
                .into_iter()
                .map(|queue| {
                    let items = store.queue_downloads(queue.id).unwrap_or_default();
                    (queue, items)
                })
                .collect()
        };
        let now = Zoned::now();
        let state = self.inner.state.lock().unwrap();
        stored
            .into_iter()
            .map(|(queue, items)| {
                let schedule = &queue.settings.schedule;
                QueueSummary {
                    queue_id: queue.id.0,
                    max_concurrent: queue.settings.max_concurrent,
                    on_complete: queue.settings.on_complete.as_str().into(),
                    running: state.queues.contains_key(&queue.id.0),
                    download_ids: items.into_iter().map(|id| id.0).collect(),
                    schedule: describe_schedule(schedule),
                    next_start: schedule
                        .next_start(&now)
                        .map(|at| at.strftime("%Y-%m-%d %H:%M").to_string()),
                    name: queue.settings.name,
                }
            })
            .collect()
    }

    /// Every queue's schedule, for the scheduler.
    pub(crate) fn queue_schedules(&self) -> Vec<(i64, Schedule)> {
        self.inner
            .store
            .lock()
            .unwrap()
            .list_queues()
            .unwrap_or_default()
            .into_iter()
            .map(|q| (q.id.0, q.settings.schedule))
            .collect()
    }

    /// A queue's name and after-queue action.
    pub(crate) fn queue_action(&self, queue: i64) -> Option<(String, PostAction)> {
        let store = self.inner.store.lock().unwrap();
        let queue = store.get_queue(QueueId(queue)).ok().flatten()?;
        Some((queue.settings.name, queue.settings.on_complete))
    }

    /// Returns when a queue was made, changed or deleted since the last call.
    pub async fn schedules_changed(&self) {
        self.inner.schedules_changed.notified().await;
    }

    pub fn create_queue(&self, new: NewQueue) -> Result<i64, String> {
        let name = check_name(&new.name)?;
        let settings = QueueSettings {
            name: name.clone(),
            max_concurrent: check_limit(new.max_concurrent.unwrap_or(DEFAULT_QUEUE_CONCURRENT))?,
            schedule: Schedule::default(),
            on_complete: match &new.on_complete {
                Some(action) => parse_action(action)?,
                None => PostAction::None,
            },
        };
        let mut store = self.inner.store.lock().unwrap();
        if find(&store, &name).is_ok() {
            return Err(format!("a queue named {name:?} already exists"));
        }
        let id = store.insert_queue(&settings).map_err(|e| e.to_string())?;
        self.inner.schedules_changed.notify_one();
        Ok(id.0)
    }

    pub fn edit_queue(&self, edit: QueueEdit) -> Result<(), String> {
        let id = {
            let mut store = self.inner.store.lock().unwrap();
            let queue = find(&store, &edit.queue)?;
            let mut settings = queue.settings;
            if let Some(name) = &edit.rename {
                let name = check_name(name)?;
                if find(&store, &name).is_ok_and(|other| other.id != queue.id) {
                    return Err(format!("a queue named {name:?} already exists"));
                }
                settings.name = name;
            }
            if let Some(limit) = edit.max_concurrent {
                settings.max_concurrent = check_limit(limit)?;
            }
            if let Some(action) = &edit.on_complete {
                settings.on_complete = parse_action(action)?;
            }
            if let Some(schedule) = &edit.schedule {
                settings.schedule = parse_schedule(schedule)?;
            }
            store
                .update_queue(queue.id, &settings)
                .map_err(|e| e.to_string())?;
            queue.id
        };
        self.inner.schedules_changed.notify_one();
        // A higher limit takes effect at once.
        self.fill_queue(id.0);
        Ok(())
    }

    /// Stops the queue, then deletes it. Its downloads stay, in no queue.
    pub fn delete_queue(&self, name: &str) -> Result<(), String> {
        let queue = find(&self.inner.store.lock().unwrap(), name)?;
        self.stop_queue_id(queue.id.0);
        self.inner
            .store
            .lock()
            .unwrap()
            .delete_queue(queue.id)
            .map_err(|e| e.to_string())?;
        self.inner.schedules_changed.notify_one();
        Ok(())
    }

    /// Starts the queue; starting a running queue changes nothing.
    pub fn start_queue(&self, name: &str) -> Result<(), String> {
        let queue = find(&self.inner.store.lock().unwrap(), name)?;
        self.start_queue_id(queue.id.0);
        Ok(())
    }

    pub(crate) fn start_queue_id(&self, queue: i64) {
        let started = match self.inner.state.lock().unwrap().queues.entry(queue) {
            Entry::Occupied(_) => false,
            Entry::Vacant(slot) => {
                slot.insert(QueueRun::default());
                true
            }
        };
        if started {
            self.fill_queue(queue);
        }
    }

    /// Stops the queue: its running downloads pause, and its after-queue action doesn't
    /// run. Stopping a queue that isn't running changes nothing.
    pub fn stop_queue(&self, name: &str) -> Result<(), String> {
        let queue = find(&self.inner.store.lock().unwrap(), name)?;
        self.stop_queue_id(queue.id.0);
        Ok(())
    }

    pub(crate) fn stop_queue_id(&self, queue: i64) {
        let mut state = self.inner.state.lock().unwrap();
        let Some(run) = state.queues.remove(&queue) else {
            return;
        };
        // Those not running yet see the run gone and don't start.
        for id in run.active {
            if let Some(cancel) = state.running.get(&id) {
                cancel.cancel();
            }
        }
    }

    /// Puts downloads at the end of a queue, taking them out of any other. A running queue
    /// starts them when it has room.
    pub fn queue_add(&self, name: &str, ids: &[i64]) -> Result<(), String> {
        let queue = {
            let mut store = self.inner.store.lock().unwrap();
            let queue = find(&store, name)?;
            for &id in ids {
                if !matches!(store.get_download(DownloadId(id)), Ok(Some(_))) {
                    return Err(format!("no download #{id}"));
                }
            }
            for &id in ids {
                store
                    .add_to_queue(queue.id, DownloadId(id))
                    .map_err(|e| e.to_string())?;
            }
            queue.id
        };
        self.fill_queue(queue.0);
        Ok(())
    }

    /// Takes downloads out of their queues. One already running carries on.
    pub fn queue_remove(&self, ids: &[i64]) -> Result<(), String> {
        let mut store = self.inner.store.lock().unwrap();
        for &id in ids {
            store
                .remove_from_queue(DownloadId(id))
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    /// Moves a download to `index` (0 = first) in its queue.
    pub fn queue_move(&self, id: i64, index: u32) -> Result<(), String> {
        let moved = self
            .inner
            .store
            .lock()
            .unwrap()
            .move_in_queue(DownloadId(id), index as usize)
            .map_err(|e| e.to_string())?;
        if moved {
            Ok(())
        } else {
            Err(format!("download #{id} isn't in a queue"))
        }
    }

    /// Starts as many of the queue's downloads as its limit allows, or ends the run when
    /// nothing is running and nothing is left to try.
    pub(super) fn fill_queue(&self, queue: i64) {
        let (limit, ready) = {
            let store = self.inner.store.lock().unwrap();
            let Ok(Some(record)) = store.get_queue(QueueId(queue)) else {
                drop(store);
                self.inner.state.lock().unwrap().queues.remove(&queue);
                return;
            };
            let ready: Vec<i64> = store
                .queue_downloads(record.id)
                .unwrap_or_default()
                .into_iter()
                .filter(|&id| {
                    matches!(
                        store.get_download(id),
                        Ok(Some(d)) if matches!(d.status, Status::Queued | Status::Paused | Status::Failed)
                    )
                })
                .map(|id| id.0)
                .collect();
            (record.settings.max_concurrent.max(1) as usize, ready)
        };

        let (picks, ended) = {
            let mut state = self.inner.state.lock().unwrap();
            let busy: Vec<i64> = ready
                .iter()
                .copied()
                .filter(|&id| state.is_busy(id))
                .collect();
            let Some(run) = state.queues.get_mut(&queue) else {
                return;
            };
            let mut picks = Vec::new();
            for id in ready {
                if run.active.len() >= limit {
                    break;
                }
                if run.tried.contains(&id) || busy.contains(&id) {
                    continue;
                }
                run.tried.insert(id);
                run.active.insert(id);
                picks.push(id);
            }
            let ended = run.active.is_empty().then_some(run.tried.len());
            if ended.is_some() {
                state.queues.remove(&queue);
            }
            (picks, ended)
        };
        if let Some(tried) = ended {
            let _ = self.inner.updates.send(Update::QueueFinished {
                queue_id: queue,
                tried,
            });
            return;
        }

        for id in picks {
            let manager = self.clone();
            tokio::spawn(async move {
                let loaded = manager.load(id).await;
                let cancel = CancellationToken::new();
                let start = {
                    let mut state = manager.inner.state.lock().unwrap();
                    let reserved = state
                        .queues
                        .get(&queue)
                        .is_some_and(|run| run.active.contains(&id));
                    match (&loaded, reserved) {
                        (Ok(_), true) => {
                            state.running.insert(id, cancel.clone());
                            true
                        }
                        // Gone meanwhile (deleted): let the queue move on.
                        (Err(_), true) => {
                            if let Some(run) = state.queues.get_mut(&queue) {
                                run.active.remove(&id);
                            }
                            false
                        }
                        // The queue was stopped, or the download paused, meanwhile.
                        (_, false) => return,
                    }
                };
                match loaded {
                    Ok(pending) if start => manager.run(pending, cancel),
                    _ => manager.fill_queue(queue),
                }
            });
        }
    }
}

/// The queue called `name`, ignoring case.
pub(super) fn find(store: &Store, name: &str) -> Result<QueueRecord, String> {
    let wanted = name.trim();
    store
        .list_queues()
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|q| q.settings.name.eq_ignore_ascii_case(wanted))
        .ok_or_else(|| format!("no queue named {wanted:?}"))
}

fn check_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("a queue needs a name".into());
    }
    if name.chars().count() > MAX_NAME_LEN {
        return Err(format!(
            "a queue name can be at most {MAX_NAME_LEN} characters"
        ));
    }
    Ok(name.into())
}

fn check_limit(limit: u32) -> Result<u32, String> {
    if (1..=MAX_QUEUE_CONCURRENT).contains(&limit) {
        Ok(limit)
    } else {
        Err(format!(
            "a queue runs 1 to {MAX_QUEUE_CONCURRENT} downloads at once"
        ))
    }
}

/// A schedule as the CLI and UI give it: times `HH:MM`, dates `YYYY-MM-DD HH:MM`.
fn parse_schedule(given: &QueueSchedule) -> Result<Schedule, String> {
    let time = |text: &str| {
        text.trim()
            .parse::<Time>()
            .map_err(|_| format!("{text:?} isn't a time; use HH:MM, like 02:00"))
    };
    let start = match (&given.start_at, &given.once) {
        (Some(_), Some(_)) => {
            return Err("a queue starts either every day at a time or once, not both".into());
        }
        (Some(at), None) => Some(Start::Daily {
            at: time(at)?,
            days: match &given.days {
                Some(days) => days.parse()?,
                None => Days::EVERY_DAY,
            },
        }),
        (None, Some(once)) => {
            let at = once
                .trim()
                .replacen(' ', "T", 1)
                .parse::<DateTime>()
                .map_err(|_| format!("{once:?} isn't a date and time; use YYYY-MM-DD HH:MM"))?;
            Some(Start::Once(at))
        }
        (None, None) => None,
    };
    if given.days.is_some() && !matches!(start, Some(Start::Daily { .. })) {
        return Err("days need a daily start time".into());
    }
    if matches!(&start, Some(Start::Daily { days, .. }) if days.is_empty()) {
        return Err("choose at least one day".into());
    }
    if given.wake && start.is_none() {
        return Err("waking the PC needs a start time".into());
    }
    Ok(Schedule {
        start,
        stop_at: given.stop_at.as_deref().map(time).transpose()?,
        wake: given.wake,
    })
}

/// The schedule in the CLI and UI's form; `None` when there is none.
fn describe_schedule(schedule: &Schedule) -> Option<QueueSchedule> {
    if schedule.start.is_none() && schedule.stop_at.is_none() {
        return None;
    }
    let hm = |t: Time| t.strftime("%H:%M").to_string();
    let mut described = QueueSchedule {
        stop_at: schedule.stop_at.map(hm),
        wake: schedule.wake,
        ..QueueSchedule::default()
    };
    match &schedule.start {
        Some(Start::Daily { at, days }) => {
            described.start_at = Some(hm(*at));
            described.days = Some(match *days {
                Days::EVERY_DAY => "daily".into(),
                Days::WEEKDAYS => "weekdays".into(),
                Days::WEEKEND => "weekend".into(),
                other => other.to_string(),
            });
        }
        Some(Start::Once(at)) => described.once = Some(at.strftime("%Y-%m-%d %H:%M").to_string()),
        None => {}
    }
    Some(described)
}

fn parse_action(name: &str) -> Result<PostAction, String> {
    PostAction::from_name(&name.trim().to_ascii_lowercase()).ok_or_else(|| {
        format!("unknown action {name:?} (use none, sleep, hibernate, shutdown or quit)")
    })
}
