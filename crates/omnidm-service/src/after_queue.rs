// SPDX-License-Identifier: GPL-3.0-or-later

//! What happens when a queue finishes: sleep, hibernate, shut down or quit OmniDm, after
//! a countdown the user can cancel.
//!
//! Only a queue that finishes by itself after trying something triggers it, never an
//! empty queue or a stopped one. If other downloads are still running, the countdown waits
//! for them; if one starts during the countdown, the action is called off. One action
//! counts down at a time; a queue finishing meanwhile adds none.

use std::sync::Arc;
use std::time::Duration;

use omnidm_ipc::protocol::{AfterQueue, AfterQueueOutcome};
use omnidm_queue::PostAction;
use omnidm_queue::power::Power;
use tokio::sync::broadcast;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use crate::manager::{Manager, Update};

/// How long the user has to cancel.
pub const COUNTDOWN: Duration = Duration::from_secs(60);
/// How often it looks whether other downloads have ended.
const WAIT_POLL: Duration = Duration::from_secs(1);

/// Quits OmniDm (the `quit` action).
pub type QuitHandler = Arc<dyn Fn() + Send + Sync>;

/// Watches for finished queues for as long as the app runs.
pub async fn run(manager: Manager, power: Arc<dyn Power>, quit: QuitHandler, countdown: Duration) {
    let mut updates = manager.subscribe();
    loop {
        let queue_id = match updates.recv().await {
            Ok(Update::QueueFinished { queue_id, tried }) if tried > 0 => queue_id,
            Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => continue,
            Err(broadcast::error::RecvError::Closed) => return,
        };
        let Some((queue, action)) = manager.queue_action(queue_id) else {
            continue;
        };
        if action == PostAction::None {
            continue;
        }
        let Some(cancel) = manager.begin_after_queue() else {
            continue;
        };
        let notice = AfterQueue {
            queue_id,
            queue,
            action: action.as_str().into(),
            seconds_left: whole_seconds(countdown),
            waiting_for_downloads: false,
            outcome: None,
            detail: None,
        };
        tokio::spawn(count_down(
            manager.clone(),
            power.clone(),
            quit.clone(),
            action,
            notice,
            countdown,
            cancel,
        ));
    }
}

async fn count_down(
    manager: Manager,
    power: Arc<dyn Power>,
    quit: QuitHandler,
    action: PostAction,
    notice: AfterQueue,
    countdown: Duration,
    cancel: CancellationToken,
) {
    let report = |left: Duration, waiting: bool, outcome, detail: Option<&str>| {
        manager.send(Update::AfterQueue(AfterQueue {
            seconds_left: whole_seconds(left),
            waiting_for_downloads: waiting,
            outcome,
            detail: detail.map(String::from),
            ..notice.clone()
        }));
    };
    let cancelled = || {
        manager.end_after_queue();
        report(
            Duration::ZERO,
            false,
            Some(AfterQueueOutcome::Cancelled),
            None,
        );
    };

    while manager.has_running() {
        report(countdown, true, None, None);
        tokio::select! {
            () = tokio::time::sleep(WAIT_POLL) => {}
            () = cancel.cancelled() => return cancelled(),
        }
    }
    let end = Instant::now() + countdown;
    loop {
        let left = end.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break;
        }
        report(left, false, None, None);
        // Wake on the second boundary, so the shown seconds tick evenly.
        let step = left - Duration::from_secs(whole_seconds(left).saturating_sub(1).into());
        tokio::select! {
            () = tokio::time::sleep(step) => {}
            () = cancel.cancelled() => return cancelled(),
        }
    }
    manager.end_after_queue();
    if manager.has_running() {
        let why = "a download started during the countdown";
        report(
            Duration::ZERO,
            false,
            Some(AfterQueueOutcome::Cancelled),
            Some(why),
        );
        return;
    }
    // Said before acting: after sleep or shutdown there may be no chance to.
    report(Duration::ZERO, false, Some(AfterQueueOutcome::Done), None);
    let result = match action {
        PostAction::Sleep => power.suspend(false),
        PostAction::Hibernate => power.suspend(true),
        PostAction::Shutdown => power.shut_down(),
        PostAction::Quit => {
            quit();
            Ok(())
        }
        PostAction::None => Ok(()),
    };
    if let Err(e) = result {
        let why = format!("Windows refused: {e}");
        report(
            Duration::ZERO,
            false,
            Some(AfterQueueOutcome::Failed),
            Some(&why),
        );
    }
}

/// Seconds, rounded up: 59.2 s left shows as 60.
fn whole_seconds(d: Duration) -> u32 {
    let seconds = d.as_secs() + u64::from(d.subsec_nanos() > 0);
    u32::try_from(seconds).unwrap_or(u32::MAX)
}
