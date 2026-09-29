//! macOS notifications for the moments that need the person: an agent
//! finished, a ticket can start, a ticket needs a human, a ticket closed.
//! Sent from a background thread; by default only while Kuzgun is not the
//! active app, as macOS expects.

use std::collections::HashMap;
use std::path::PathBuf;

use crate::agents::RunState;
use crate::model::{Board, Category, Mode};

/// What the board looked like for one ticket, to spot changes.
#[derive(Clone, PartialEq)]
pub struct Snap {
    pub key: String,
    pub title: String,
    pub category: Category,
    pub frontier: bool,
    pub agent: Option<RunState>,
    pub hitl: bool,
}

pub fn snapshot(
    board: &Board,
    frontier: &[bool],
    modes: &[Option<Mode>],
) -> HashMap<PathBuf, Snap> {
    board
        .tickets
        .iter()
        .enumerate()
        .map(|(i, t)| {
            (
                t.path.clone(),
                Snap {
                    key: t.key.clone(),
                    title: t.title.clone(),
                    category: t.category,
                    frontier: frontier.get(i).copied().unwrap_or(false),
                    agent: t.agent.as_ref().map(|a| a.state),
                    hitl: modes.get(i).copied().flatten() == Some(Mode::Hitl),
                },
            )
        })
        .collect()
}

/// (title, body) for each change worth a notification, per the settings.
pub fn changes(
    old: &HashMap<PathBuf, Snap>,
    new: &HashMap<PathBuf, Snap>,
) -> Vec<(String, String)> {
    let p = crate::settings::get();
    let mut out = Vec::new();
    for (path, n) in new {
        let Some(o) = old.get(path) else {
            continue;
        };
        let name = format!("{} {}", n.key, n.title);
        match (o.agent, n.agent) {
            (Some(RunState::Running), Some(RunState::AwaitingReview | RunState::Finished))
                if p.notify_agent_done =>
            {
                out.push((
                    "An agent finished".into(),
                    format!("{name} is ready for review."),
                ));
                continue;
            }
            (None | Some(RunState::Finished), Some(RunState::Running))
                if p.notify_agent_started =>
            {
                out.push(("An agent started".into(), format!("Working on {name}.")));
                continue;
            }
            _ => {}
        }
        if !o.category.is_closed() && n.category.is_closed() && p.notify_closed {
            out.push((format!("{} {}", n.key, n.category.label()), n.title.clone()));
        } else if !o.frontier && n.frontier {
            if n.hitl && p.notify_needs_you {
                out.push((
                    "Needs you".into(),
                    format!("{name} can start, and it needs a person."),
                ));
            } else if p.notify_unblocked {
                out.push((
                    "Ready to start".into(),
                    format!("{name}: its last blocker closed."),
                ));
            }
        }
    }
    out
}

/// Posts one notification (grouped into one when there are many).
pub fn post(items: Vec<(String, String)>) {
    if items.is_empty() || !crate::settings::get().notify_enabled {
        return;
    }
    let (title, body) = if items.len() > 3 {
        (
            format!("{} tickets changed", items.len()),
            items
                .iter()
                .map(|(t, b)| format!("{t}: {b}"))
                .take(4)
                .collect::<Vec<_>>()
                .join("\n"),
        )
    } else {
        items[0].clone()
    };
    let rest: Vec<(String, String)> = if items.len() > 3 {
        Vec::new()
    } else {
        items[1..].to_vec()
    };
    std::thread::spawn(move || {
        for (t, b) in std::iter::once((title, body)).chain(rest) {
            if let Err(e) = send(&t, &b) {
                log::warn!("notification failed: {e}");
            }
        }
    });
}

/// Asks macOS for permission at launch, so the prompt shows up before the
/// first event and not in the middle of one.
pub fn request_permission() {
    #[cfg(target_os = "macos")]
    if std::env::current_exe().is_ok_and(|p| p.to_string_lossy().contains(".app/Contents/MacOS/")) {
        mac::request_permission();
    }
}

/// Shows one OS notification now.
pub fn send(title: &str, body: &str) -> Result<(), String> {
    // Inside Kuzgun.app macOS takes the modern API; the old one that
    // notify-rust calls shows nothing on current macOS.
    #[cfg(target_os = "macos")]
    if std::env::current_exe().is_ok_and(|p| p.to_string_lossy().contains(".app/Contents/MacOS/")) {
        return mac::send(title, body);
    }
    notify_rust::Notification::new()
        .appname("Kuzgun")
        .summary(title)
        .body(body)
        .show()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[cfg(target_os = "macos")]
mod mac {
    use std::sync::Once;

    use block2::RcBlock;
    use objc2::runtime::Bool;
    use objc2_foundation::{NSError, NSString};
    use objc2_user_notifications::{
        UNAuthorizationOptions, UNMutableNotificationContent, UNNotificationRequest,
        UNNotificationSound, UNUserNotificationCenter,
    };

    static ASK: Once = Once::new();

    /// Asks macOS once per launch; the system shows its prompt only the
    /// first time and remembers the answer.
    pub fn request_permission() {
        ASK.call_once(|| {
            let center = UNUserNotificationCenter::currentNotificationCenter();
            let done = RcBlock::new(|granted: Bool, err: *mut NSError| {
                if !granted.as_bool() {
                    let why = unsafe { err.as_ref() }
                        .map(|e| e.localizedDescription().to_string())
                        .unwrap_or_default();
                    log::warn!("notifications not allowed {why}");
                }
            });
            center.requestAuthorizationWithOptions_completionHandler(
                UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound,
                &done,
            );
        });
    }

    pub fn send(title: &str, body: &str) -> Result<(), String> {
        request_permission();
        let content = UNMutableNotificationContent::new();
        content.setTitle(&NSString::from_str(title));
        content.setBody(&NSString::from_str(body));
        content.setSound(Some(&UNNotificationSound::defaultSound()));
        let id = NSString::from_str(&format!(
            "kuzgun-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let request =
            UNNotificationRequest::requestWithIdentifier_content_trigger(&id, &content, None);
        let done = RcBlock::new(|err: *mut NSError| {
            if let Some(e) = unsafe { err.as_ref() } {
                log::warn!("notification failed: {}", e.localizedDescription());
            }
        });
        UNUserNotificationCenter::currentNotificationCenter()
            .addNotificationRequest_withCompletionHandler(&request, Some(&done));
        Ok(())
    }
}
