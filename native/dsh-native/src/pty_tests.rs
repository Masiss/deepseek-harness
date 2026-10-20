//! PTY reader tests over the byte boundaries the terminal produces.

use crate::utf8_complete_prefix;

/// A code point split across two reads must stay one character.
#[test]
fn a_split_code_point_is_decoded_once() {
    let text = "héllo";
    let bytes = text.as_bytes();
    // The first read ends inside the two-byte sequence of "é".
    let first = &bytes[..2];
    let second = &bytes[2..];
    assert_eq!(first.len(), 2);
    assert!(first[1] & 0b1100_0000 == 0b1100_0000, "the first read ends on the lead byte");

    let mut pending = first.to_vec();
    let complete = utf8_complete_prefix(&pending);
    assert_eq!(complete, 1, "the incomplete sequence is not decoded yet");
    pending.extend_from_slice(second);
    let complete = utf8_complete_prefix(&pending);
    assert_eq!(complete, pending.len());
    assert_eq!(String::from_utf8_lossy(&pending), text);
}

/// Complete text decodes in one piece.
#[test]
fn complete_text_is_decoded_whole() {
    let bytes = "ready".as_bytes();
    assert_eq!(utf8_complete_prefix(bytes), bytes.len());
}

/// An invalid sequence is reported as it is instead of stalling the reader.
#[test]
fn an_invalid_sequence_does_not_stall() {
    let bytes = [0x41, 0xC3, 0x28, 0x42];
    assert_eq!(utf8_complete_prefix(&bytes), bytes.len());
}
