// SPDX-License-Identifier: GPL-3.0-or-later

//! The scheduler, driven by hand-picked clock times instead of the real clock, and the
//! after-queue actions, both with a stand-in for the PC's power management.

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use jiff::{Timestamp, Zoned};
use omnidm_core::engine::EngineConfig;
use omnidm_core::{RampMode, RampPolicy};
use omnidm_ipc::protocol::{AddRequest, AfterQueue, AfterQueueOutcome, QueueEdit, QueueSchedule};
use omnidm_queue::power::Power;
use omnidm_service::manager::{Manager, ManagerConfig, Update};
use omnidm_service::scheduler::{Scheduler, WAKE_EARLY};
use omnidm_testserver::{FileSpec, TestServer};

const KIB: u64 = 1024;
const MIB: u64 = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Call {
    Wake(Option<Timestamp>),
    Awake(bool),
    Suspend { hibernate: bool },
    ShutDown,
}

#[derive(Default)]
struct FakePower {
    calls: Mutex<Vec<Call>>,
}

impl FakePower {
    fn take(&self) -> Vec<Call> {
        std::mem::take(&mut self.calls.lock().unwrap())
    }
}

impl Power for FakePower {
    fn wake_at(&self, at: Option<Timestamp>) -> io::Result<()> {
        self.calls.lock().unwrap().push(Call::Wake(at));
        Ok(())
    }

    fn keep_awake(&self, on: bool) -> io::Result<()> {
        self.calls.lock().unwrap().push(Call::Awake(on));
        Ok(())
    }

    fn suspend(&self, hibernate: bool) -> io::Result<()> {
        self.calls.lock().unwrap().push(Call::Suspend { hibernate });
        Ok(())
    }

    fn shut_down(&self) -> io::Result<()> {
        self.calls.lock().unwrap().push(Call::ShutDown);
        Ok(())
    }
}

struct Env {
    server: TestServer,
    manager: Manager,
    power: Arc<FakePower>,
    scheduler: Scheduler,
    _data: tempfile::TempDir,
    downloads: tempfile::TempDir,
}

async fn env() -> Env {
    let server = TestServer::start().await.unwrap();
    let data = tempfile::tempdir().unwrap();
    let downloads = tempfile::tempdir().unwrap();
    let manager = Manager::start(ManagerConfig {
        data_dir: data.path().to_path_buf(),
        downloads_dir: downloads.path().to_path_buf(),
        max_active: 3,
        engine: EngineConfig {
            ramp: RampPolicy {
                max_connections: 1,
                mode: RampMode::Doubling,
                cooldown: Duration::from_millis(100),
            },
            ..EngineConfig::default()
        },
        probe_timeout: Duration::from_secs(5),
    })
    .unwrap();
    let power = Arc::new(FakePower::default());
    let scheduler = Scheduler::new(manager.clone(), power.clone());
    Env {
        server,
        manager,
        power,
        scheduler,
        _data: data,
        downloads,
    }
}

/// New York time, so the tests don't depend on the PC's time zone.
fn at(text: &str) -> Zoned {
    format!("{text}[America/New_York]").parse().unwrap()
}

impl Env {
    /// Puts a download in Main that keeps running for a long while.
    fn queue_long_download(&self) -> i64 {
        self.server.set_file(
            "/long.bin",
            FileSpec::new(64 * MIB).etag("long").throttle(64 * KIB),
        );
        self.manager
            .add(AddRequest {
                url: self.server.url("/long.bin"),
                dir: self.downloads.path().to_string_lossy().into_owned(),
                file_name: None,
                connections: None,
                headers: vec![],
                cookies: vec![],
                data: None,
                queue: Some("Main".into()),
            })
            .unwrap()
    }

    fn schedule(&self, schedule: QueueSchedule) -> Result<(), String> {
        self.manager.edit_queue(QueueEdit {
            queue: "Main".into(),
            rename: None,
            max_concurrent: None,
            on_complete: None,
            schedule: Some(schedule),
        })
    }

    fn main_running(&self) -> bool {
        self.manager.queues()[0].running
    }
}

fn daily(start: &str, stop: Option<&str>) -> QueueSchedule {
    QueueSchedule {
        start_at: Some(start.into()),
        stop_at: stop.map(String::from),
        ..QueueSchedule::default()
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_queue_starts_and_stops_at_its_times() {
    let mut env = env().await;
    env.queue_long_download();
    env.schedule(daily("02:00", Some("07:00"))).unwrap();

    let next = env.scheduler.tick(&at("2026-06-10T01:00"));
    assert_eq!(next, Some(at("2026-06-10T02:00").timestamp()));
    env.scheduler.tick(&at("2026-06-10T01:59:59"));
    assert!(!env.main_running());

    let next = env.scheduler.tick(&at("2026-06-10T02:00:01"));
    assert!(env.main_running(), "started at 02:00");
    assert_eq!(next, Some(at("2026-06-10T07:00").timestamp()));

    env.scheduler.tick(&at("2026-06-10T06:59"));
    assert!(env.main_running());
    env.scheduler.tick(&at("2026-06-10T07:00:01"));
    assert!(!env.main_running(), "stopped at 07:00");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_start_missed_while_asleep_runs_on_waking() {
    let mut env = env().await;
    env.queue_long_download();
    env.schedule(daily("02:00", None)).unwrap();
    env.scheduler.tick(&at("2026-06-10T01:00"));
    // Asleep until 05:00, with no wake timer.
    env.scheduler.tick(&at("2026-06-10T05:00"));
    assert!(env.main_running());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_start_and_stop_both_missed_leave_the_queue_stopped() {
    let mut env = env().await;
    env.queue_long_download();
    env.schedule(daily("02:00", Some("07:00"))).unwrap();
    env.scheduler.tick(&at("2026-06-10T01:00"));
    // Asleep from 01:00 to 08:00: the 07:00 stop came after the 02:00 start.
    env.scheduler.tick(&at("2026-06-10T08:00"));
    assert!(!env.main_running());
}

#[tokio::test(flavor = "multi_thread")]
async fn times_before_the_first_tick_are_not_caught_up_on() {
    let mut env = env().await;
    env.queue_long_download();
    env.schedule(daily("02:00", None)).unwrap();
    // OmniDm started at 03:00, after the start time.
    env.scheduler.tick(&at("2026-06-10T03:00"));
    assert!(!env.main_running());
}

#[tokio::test(flavor = "multi_thread")]
async fn the_pc_is_woken_a_minute_early_only_when_asked() {
    let mut env = env().await;
    env.schedule(daily("02:00", None)).unwrap();
    env.scheduler.tick(&at("2026-06-10T01:00"));
    assert!(
        !env.power.take().iter().any(|c| matches!(c, Call::Wake(_))),
        "no wake timer without --wake"
    );

    env.schedule(QueueSchedule {
        wake: true,
        ..daily("02:00", None)
    })
    .unwrap();
    env.scheduler.tick(&at("2026-06-10T01:00:10"));
    let wake = at("2026-06-10T02:00").timestamp() - WAKE_EARLY;
    assert_eq!(env.power.take(), [Call::Wake(Some(wake))]);

    // Unchanged: not set again.
    env.scheduler.tick(&at("2026-06-10T01:00:20"));
    assert_eq!(env.power.take(), []);

    env.schedule(daily("02:00", None)).unwrap();
    env.scheduler.tick(&at("2026-06-10T01:00:30"));
    assert_eq!(env.power.take(), [Call::Wake(None)], "cancelled");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_pc_is_kept_awake_while_downloads_run() {
    let mut env = env().await;
    env.scheduler.tick(&at("2026-06-10T01:00"));
    assert_eq!(env.power.take(), [Call::Awake(false)]);

    let id = env.queue_long_download();
    env.manager.start_queue("Main").unwrap();
    // The queue starts its download on another task.
    for _ in 0..50 {
        if env.manager.has_running() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    env.scheduler.tick(&at("2026-06-10T01:00:10"));
    assert_eq!(env.power.take(), [Call::Awake(true)]);

    env.manager.pause(id);
    for _ in 0..50 {
        if !env.manager.has_running() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    env.scheduler.tick(&at("2026-06-10T01:00:20"));
    assert_eq!(env.power.take(), [Call::Awake(false)]);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_running_scheduler_starts_a_queue_on_the_real_clock() {
    let env = env().await;
    env.queue_long_download();
    tokio::spawn(omnidm_service::scheduler::run(
        env.manager.clone(),
        Arc::new(omnidm_queue::power::NoPower),
    ));
    tokio::time::sleep(Duration::from_millis(200)).await;
    // Set after the scheduler's first look, as a user would.
    let soon = Zoned::now() + jiff::SignedDuration::from_secs(2);
    env.schedule(QueueSchedule {
        once: Some(soon.strftime("%Y-%m-%d %H:%M:%S").to_string()),
        ..QueueSchedule::default()
    })
    .unwrap();
    assert!(!env.main_running());
    let started = tokio::time::timeout(Duration::from_secs(6), async {
        while !env.main_running() {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await;
    assert!(started.is_ok(), "the queue didn't start on time");
    env.manager.stop_queue("Main").unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn schedules_are_checked_and_shown() {
    let env = env().await;
    let bad = |schedule: QueueSchedule| env.schedule(schedule).unwrap_err();
    assert!(bad(daily("2am", None)).contains("isn't a time"));
    assert!(
        bad(QueueSchedule {
            once: Some("2026-06-10 02:00".into()),
            ..daily("02:00", None)
        })
        .contains("not both")
    );
    assert!(
        bad(QueueSchedule {
            days: Some("mon".into()),
            ..QueueSchedule::default()
        })
        .contains("days need")
    );
    assert!(
        bad(QueueSchedule {
            wake: true,
            ..QueueSchedule::default()
        })
        .contains("needs a start time")
    );
    assert!(
        bad(QueueSchedule {
            once: Some("tomorrow".into()),
            ..QueueSchedule::default()
        })
        .contains("isn't a date")
    );

    env.schedule(QueueSchedule {
        days: Some("weekdays".into()),
        wake: true,
        ..daily("02:00", Some("07:30"))
    })
    .unwrap();
    let main = env.manager.queues().remove(0);
    let shown = main.schedule.unwrap();
    assert_eq!(shown.start_at.as_deref(), Some("02:00"));
    assert_eq!(shown.days.as_deref(), Some("weekdays"));
    assert_eq!(shown.stop_at.as_deref(), Some("07:30"));
    assert!(shown.wake);
    assert!(main.next_start.unwrap().ends_with("02:00"));

    env.schedule(QueueSchedule {
        once: Some("2099-01-02 23:30".into()),
        ..QueueSchedule::default()
    })
    .unwrap();
    let main = env.manager.queues().remove(0);
    assert_eq!(
        main.schedule.unwrap().once.as_deref(),
        Some("2099-01-02 23:30")
    );
    assert_eq!(main.next_start.as_deref(), Some("2099-01-02 23:30"));

    env.schedule(QueueSchedule::default()).unwrap();
    let main = env.manager.queues().remove(0);
    assert_eq!((main.schedule, main.next_start), (None, None));
}

// --- after-queue actions ---

/// A quick countdown, so the tests don't wait a minute.
const SHORT: Duration = Duration::from_millis(300);

impl Env {
    /// Adds a small file to `queue` (or starts it now, given `None`).
    fn add_small(&self, queue: Option<&str>, path: &str) -> i64 {
        self.server
            .set_file(path, FileSpec::new(64 * KIB).etag(path));
        self.manager
            .add(AddRequest {
                url: self.server.url(path),
                dir: self.downloads.path().to_string_lossy().into_owned(),
                file_name: None,
                connections: None,
                headers: vec![],
                cookies: vec![],
                data: None,
                queue: queue.map(String::from),
            })
            .unwrap()
    }

    fn then(&self, action: &str) {
        self.manager
            .edit_queue(QueueEdit {
                queue: "Main".into(),
                rename: None,
                max_concurrent: None,
                on_complete: Some(action.into()),
                schedule: None,
            })
            .unwrap();
    }

    /// Runs the after-queue watcher; the returned flag is set if OmniDm is told to quit.
    fn watch_after_queue(&self, countdown: Duration) -> Arc<AtomicBool> {
        let quit = Arc::new(AtomicBool::new(false));
        let flag = quit.clone();
        tokio::spawn(omnidm_service::after_queue::run(
            self.manager.clone(),
            self.power.clone(),
            Arc::new(move || flag.store(true, Ordering::SeqCst)),
            countdown,
        ));
        quit
    }
}

/// The next after-queue notice, or `None` if none comes within `wait`.
async fn notice(
    updates: &mut tokio::sync::broadcast::Receiver<Update>,
    wait: Duration,
) -> Option<AfterQueue> {
    tokio::time::timeout(wait, async {
        loop {
            if let Ok(Update::AfterQueue(notice)) = updates.recv().await {
                return notice;
            }
        }
    })
    .await
    .ok()
}

/// Notices until the outcome; returns them all.
async fn until_outcome(updates: &mut tokio::sync::broadcast::Receiver<Update>) -> Vec<AfterQueue> {
    let mut seen = Vec::new();
    loop {
        let next = notice(updates, Duration::from_secs(20))
            .await
            .expect("no after-queue outcome");
        let done = next.outcome.is_some();
        seen.push(next);
        if done {
            return seen;
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn each_action_runs_after_the_countdown() {
    let env = env().await;
    let quit = env.watch_after_queue(SHORT);
    let mut updates = env.manager.subscribe();
    let actions = [
        ("sleep", Some(Call::Suspend { hibernate: false })),
        ("hibernate", Some(Call::Suspend { hibernate: true })),
        ("shutdown", Some(Call::ShutDown)),
        ("quit", None),
    ];
    for (n, (action, call)) in actions.into_iter().enumerate() {
        env.then(action);
        env.add_small(Some("Main"), &format!("/{action}.bin"));
        env.manager.start_queue("Main").unwrap();
        let seen = until_outcome(&mut updates).await;
        let last = seen.last().unwrap();
        assert_eq!(last.outcome, Some(AfterQueueOutcome::Done), "{action}");
        assert_eq!(
            (last.queue.as_str(), last.action.as_str()),
            ("Main", action)
        );
        assert!(
            seen[..seen.len() - 1]
                .iter()
                .all(|s| s.outcome.is_none() && s.seconds_left == 1),
            "counted down first: {seen:?}"
        );
        // "Done" is said just before acting.
        settle(|| call.is_none() || !env.power.calls.lock().unwrap().is_empty()).await;
        assert_eq!(
            env.power.take(),
            call.into_iter().collect::<Vec<_>>(),
            "#{n}"
        );
    }
    settle(|| quit.load(Ordering::SeqCst)).await;
    assert!(quit.load(Ordering::SeqCst));
}

/// Waits up to 2 s for `ready`.
async fn settle(ready: impl Fn() -> bool) {
    for _ in 0..100 {
        if ready() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_countdown_can_be_cancelled() {
    let env = env().await;
    env.watch_after_queue(Duration::from_secs(30));
    let mut updates = env.manager.subscribe();
    env.then("shutdown");
    env.add_small(Some("Main"), "/a.bin");
    env.manager.start_queue("Main").unwrap();

    let first = notice(&mut updates, Duration::from_secs(20)).await.unwrap();
    assert_eq!((first.seconds_left, first.outcome), (30, None));
    assert!(env.manager.cancel_after_queue());
    let seen = until_outcome(&mut updates).await;
    assert_eq!(
        seen.last().unwrap().outcome,
        Some(AfterQueueOutcome::Cancelled)
    );
    assert_eq!(env.power.take(), []);
    assert!(!env.manager.cancel_after_queue(), "nothing left to cancel");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_empty_queue_or_no_action_does_nothing() {
    let env = env().await;
    env.watch_after_queue(SHORT);
    let mut updates = env.manager.subscribe();
    env.then("shutdown");
    // Empty: it finishes having tried nothing.
    env.manager.start_queue("Main").unwrap();
    assert_eq!(notice(&mut updates, Duration::from_millis(800)).await, None);

    env.then("none");
    env.add_small(Some("Main"), "/a.bin");
    env.manager.start_queue("Main").unwrap();
    assert_eq!(notice(&mut updates, Duration::from_secs(2)).await, None);
    assert_eq!(env.power.take(), []);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_action_waits_for_other_downloads() {
    let env = env().await;
    env.watch_after_queue(SHORT);
    let mut updates = env.manager.subscribe();
    env.then("sleep");
    // Running outside the queue.
    env.server.set_file(
        "/other.bin",
        FileSpec::new(64 * MIB).etag("other").throttle(64 * KIB),
    );
    let other = env
        .manager
        .add(AddRequest {
            url: env.server.url("/other.bin"),
            dir: env.downloads.path().to_string_lossy().into_owned(),
            file_name: None,
            connections: None,
            headers: vec![],
            cookies: vec![],
            data: None,
            queue: None,
        })
        .unwrap();
    env.add_small(Some("Main"), "/a.bin");
    env.manager.start_queue("Main").unwrap();

    let waiting = notice(&mut updates, Duration::from_secs(20)).await.unwrap();
    assert!(waiting.waiting_for_downloads);
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(env.power.take(), [], "not while the other runs");

    env.manager.pause(other);
    let seen = until_outcome(&mut updates).await;
    assert_eq!(seen.last().unwrap().outcome, Some(AfterQueueOutcome::Done));
    settle(|| !env.power.calls.lock().unwrap().is_empty()).await;
    assert_eq!(env.power.take(), [Call::Suspend { hibernate: false }]);
}
