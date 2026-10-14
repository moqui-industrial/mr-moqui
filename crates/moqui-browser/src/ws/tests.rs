/*
 * This software is in the public domain under CC0 1.0 Universal plus a
 * Grant of Patent License.
 *
 * To the extent possible under law, the author(s) have dedicated all
 * copyright and related and neighboring rights to this software to the
 * public domain worldwide. This software is distributed without any
 * warranty.
 *
 * You should have received a copy of the CC0 Public Domain Dedication
 * along with this software (see the LICENSE.md file). If not, see
 * <http://creativecommons.org/publicdomain/zero/1.0/>.
 */

use std::sync::{Arc, Mutex};

use moqui_core::notify::{subscribe, TopicMessage, NOTIFICATION_MESSAGE};
use serde_json::json;

use super::*;

/// The topic bus is process-wide and tests run in parallel: each test uses its own message ids.
fn unique(label: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    format!(
        "{label}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

fn listen() -> (moqui_core::notify::Subscription, Arc<Mutex<Vec<TopicMessage>>>) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&seen);
    let subscription = subscribe(
        NOTIFICATION_MESSAGE,
        Arc::new(move |message: &TopicMessage| sink.lock().unwrap().push(message.clone())),
    )
    .unwrap();
    (subscription, seen)
}

fn seen_ids(seen: &Mutex<Vec<TopicMessage>>, id: &str) -> Vec<TopicMessage> {
    seen.lock()
        .unwrap()
        .iter()
        .filter(|message| message.message_id == id)
        .cloned()
        .collect()
}

#[test]
fn a_notification_is_published_locally_once_without_its_routing() {
    let (_subscription, seen) = listen();
    let id = unique("note");
    let frame = json!({
        "topic": "Wave6",
        "notificationMessageId": id,
        "title": "Disk full",
        "userIds": ["stale"],
    })
    .to_string();
    let delivered = deliver_text("notws", &frame).unwrap();
    let Delivered::Notification(body) = delivered else {
        panic!("not a notification: {delivered:?}");
    };
    assert_eq!(body["title"], json!("Disk full"));
    assert!(body.get("userIds").is_none(), "{body}");
    // The bus delivers a repeated notificationMessageId once.
    deliver_text("notws", &frame).unwrap();
    let got = seen_ids(&seen, &id);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].origin_id, "ws:notws");
    assert_eq!(got[0].body["topic"], json!("Wave6"));
}

#[test]
fn other_text_is_returned_and_not_published() {
    let (_subscription, seen) = listen();
    for text in ["pong", "[1,2]", r#"{"hello":"world"}"#] {
        assert_eq!(deliver_text("notws", text).unwrap(), Delivered::Text(text.to_string()));
    }
    assert!(seen
        .lock()
        .unwrap()
        .iter()
        .all(|message| !message.origin_id.starts_with("ws:") || message.body.get("hello").is_none()));
}

#[test]
fn binary_frames_wait_behind_handles_and_the_oldest_is_dropped() {
    let first = deliver_binary("msg", vec![1, 2, 3]);
    let second = deliver_binary("msg", vec![4]);
    assert_ne!(first, 0);
    assert_ne!(first, second);
    assert_eq!(pending_binary(), vec![first, second]);
    assert_eq!(
        take_binary(first),
        Some(InboundBinary {
            conn: "msg".to_string(),
            bytes: vec![1, 2, 3],
        })
    );
    assert_eq!(take_binary(first), None);
    for _ in 0..BINARY_FRAMES {
        deliver_binary("msg", vec![0]);
    }
    assert_eq!(pending_binary().len(), BINARY_FRAMES);
    assert!(
        take_binary(second).is_none(),
        "the oldest frame should have been dropped"
    );
    assert_eq!(dropped_binary(), 1);
}

#[test]
fn outbound_frames_drain_in_order_and_a_full_queue_refuses() {
    send_text("notws", "subscribe:ALL").unwrap();
    send_bytes("msg", vec![9, 8]).unwrap();
    assert_eq!(outbox_len(), 2);
    assert_eq!(
        take_outbox(),
        vec![
            Outbound::Text {
                conn: "notws".to_string(),
                text: "subscribe:ALL".to_string(),
            },
            Outbound::Binary {
                conn: "msg".to_string(),
                bytes: vec![9, 8],
            },
        ]
    );
    assert!(take_outbox().is_empty());
    for _ in 0..OUTBOX_FRAMES {
        send_text("notws", "x").unwrap();
    }
    assert!(send_text("notws", "one too many").is_err());
    assert_eq!(take_outbox().len(), OUTBOX_FRAMES);
}
