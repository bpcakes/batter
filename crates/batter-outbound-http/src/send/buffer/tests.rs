//! Growth, limits and final ownership of the wiping buffer.
#![allow(clippy::missing_assert_message, reason = "test names locate failures")]

use super::{ZeroizingBuffer, probe};
use bytes::{Bytes, BytesMut};

const LIMIT: usize = 64;

fn filled(bytes: &[u8]) -> ZeroizingBuffer {
    let mut buffer = ZeroizingBuffer::with_capacity(bytes.len());
    buffer.append(bytes, LIMIT).unwrap();
    buffer
}

#[test]
fn growth_wipes_every_replaced_allocation_and_keeps_exact_bytes() {
    probe::take();
    let mut buffer = ZeroizingBuffer::with_capacity(4);
    buffer.append(b"abcd", LIMIT).unwrap();
    assert!(probe::take().is_empty(), "a fitting chunk replaces nothing");
    buffer.append(b"ef", LIMIT).unwrap();
    assert_eq!(probe::take(), [probe::wiped(b"abcd", 4)]);
    buffer.append(b"ghijklmnop", LIMIT).unwrap();
    assert_eq!(probe::take(), [probe::wiped(b"abcdef", 8)]);
    assert_eq!(buffer.into_bytes(), "abcdefghijklmnop");
    assert_eq!(probe::take(), [probe::wiped(b"abcdefghijklmnop", 16)]);
}

#[test]
fn growth_stays_within_the_limit_and_a_refused_chunk_is_not_stored() {
    probe::take();
    let mut buffer = ZeroizingBuffer::with_capacity(0);
    buffer.append(b"abcdef", 10).unwrap();
    buffer.append(b"ghi", 10).unwrap();
    // Doubling six bytes would make twelve; the limit holds it to ten.
    assert_eq!(probe::take(), [probe::wiped(b"abcdef", 6)]);
    buffer.append(b"j", 10).unwrap();
    assert!(buffer.append(b"k", 10).is_err());
    assert!(probe::take().is_empty(), "a refused chunk must not grow");
    drop(buffer);
    assert_eq!(probe::take(), [probe::wiped(b"abcdefghij", 10)]);
}

/// A completed body has at least filled its first allocation, so this is the
/// documented bound on what a returned body keeps allocated.
#[test]
fn a_body_past_its_first_allocation_holds_at_most_twice_its_length() {
    const CEILING: usize = 4096;
    for first in [0, 1, 7, 64] {
        for step in [1, 2, 3, 5, 13, 97] {
            let chunk = vec![1; step];
            let mut buffer = ZeroizingBuffer::with_capacity(first);
            while buffer.filled + step <= CEILING {
                buffer.append(&chunk, CEILING).unwrap();
                if buffer.filled >= first {
                    assert!(
                        buffer.storage.len() <= 2 * buffer.filled,
                        "first {first}, step {step}: {} allocated for {} held",
                        buffer.storage.len(),
                        buffer.filled
                    );
                }
            }
        }
    }
}

/// Production never writes past `filled`. Dirtying that tail here is what tells
/// a whole-allocation wipe from one bounded by the written length.
#[test]
fn dropping_a_partly_filled_buffer_wipes_the_whole_allocation() {
    probe::take();
    let mut buffer = ZeroizingBuffer::with_capacity(32);
    buffer.append(b"partial", LIMIT).unwrap();
    buffer.storage[31] = 0xA5;
    drop(buffer);
    assert_eq!(probe::take(), [probe::wiped(b"partial", 32)]);
}

#[test]
fn a_returned_body_exposes_only_its_bytes_and_keeps_its_allocation_until_release() {
    probe::take();
    let mut buffer = ZeroizingBuffer::with_capacity(32);
    buffer.append(b"exact", LIMIT).unwrap();
    let bytes = buffer.into_bytes();
    assert!(probe::take().is_empty(), "completion must not reallocate");
    assert_eq!(bytes, "exact");
    drop(bytes);
    assert_eq!(probe::take(), [probe::wiped(b"exact", 32)]);
}

#[test]
fn every_release_order_keeps_the_remaining_views_intact_until_the_last() {
    for order in [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ] {
        probe::take();
        let body = filled(b"whole-body").into_bytes();
        let clone = body.clone();
        let slice = body.slice(6..);
        let mut views = [
            Some((body, &b"whole-body"[..])),
            Some((clone, &b"whole-body"[..])),
            Some((slice, &b"body"[..])),
        ];
        let (last, earlier) = order.split_last().unwrap();
        for index in earlier {
            views[*index] = None;
            assert!(probe::take().is_empty(), "a remaining view keeps the owner");
            for (view, expected) in views.iter().flatten() {
                assert_eq!(&view[..], *expected);
            }
        }
        views[*last] = None;
        assert_eq!(probe::take(), [probe::wiped(b"whole-body", 10)]);
    }
}

/// The last view is released on another thread, which is where its record is.
#[test]
fn a_returned_body_stays_intact_until_its_last_clone_or_slice_is_released() {
    probe::take();
    let mut bytes = filled(b"head-body-tail").into_bytes();
    let clone = bytes.clone();
    let head = bytes.split_to(4);
    let slice = clone.slice(5..9);
    drop(bytes);
    assert_eq!(head, "head");
    drop(head);
    assert_eq!(clone, "head-body-tail");
    drop(clone);
    assert!(probe::take().is_empty(), "a remaining view keeps the owner");
    assert_eq!(slice, "body");
    let released = std::thread::spawn(move || {
        drop(slice);
        probe::take()
    })
    .join()
    .unwrap();
    assert_eq!(released, [probe::wiped(b"head-body-tail", 14)]);
    assert!(probe::take().is_empty(), "one owner is released once");
}

/// A returned body is never unique, even through its only handle, so `Vec` and
/// `BytesMut` conversions copy. The copy is the caller's to wipe, and the
/// conversion still releases its share of the owner.
#[test]
fn conversions_copy_out_and_release_their_share_of_the_owner() {
    probe::take();
    let only = filled(b"copied").into_bytes();
    assert!(!only.is_unique());
    let bytes = only
        .try_into_mut()
        .expect_err("an owned body is never unique");
    let clone = bytes.clone();
    let vector = Vec::from(bytes);
    assert!(probe::take().is_empty(), "a remaining view keeps the owner");
    let mutable = BytesMut::from(clone);
    assert_eq!(probe::take(), [probe::wiped(b"copied", 6)]);
    assert_eq!(vector, b"copied");
    assert_eq!(mutable, b"copied"[..]);
}

#[test]
fn an_empty_body_has_nothing_to_wipe() {
    probe::take();
    let bytes = ZeroizingBuffer::with_capacity(0).into_bytes();
    assert_eq!(bytes, Bytes::new());
    drop(bytes);
    assert!(probe::take().is_empty());
}
