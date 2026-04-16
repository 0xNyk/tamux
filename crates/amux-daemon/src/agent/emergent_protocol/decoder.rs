pub(crate) fn classify_pattern_key(input: &str) -> Option<super::types::ProtocolSignalKind> {
    let normalized = input.trim();
    if normalized.is_empty() {
        return None;
    }

    if normalized.contains(" -> ")
        && normalized
            .split(" -> ")
            .all(|segment| !segment.trim().is_empty())
    {
        return Some(super::types::ProtocolSignalKind::RepeatedShorthand);
    }

    if matches!(normalized, "continue" | "go on" | "keep going") {
        return Some(super::types::ProtocolSignalKind::RepeatedContinuationCue);
    }

    if matches!(normalized, "ok" | "okay" | "sgtm" | "sounds good") {
        return Some(super::types::ProtocolSignalKind::RepeatedAffirmation);
    }

    if normalized.len() <= 24 && normalized.split_whitespace().count() <= 3 {
        return Some(super::types::ProtocolSignalKind::RepeatedShorthand);
    }

    None
}
