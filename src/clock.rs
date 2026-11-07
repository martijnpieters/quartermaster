use chrono::{DateTime, FixedOffset, Local, SecondsFormat, Utc};

/// `Time.now.iso8601`: local offset, whole seconds.
pub fn now_iso() -> String {
    Local::now().to_rfc3339_opts(SecondsFormat::Secs, false)
}

/// `Time.now.utc.iso8601`.
pub fn now_utc_iso() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

pub fn today() -> String {
    Local::now().format("%Y-%m-%d").to_string()
}

pub fn parse(ts: &str) -> Option<DateTime<FixedOffset>> {
    DateTime::parse_from_rfc3339(ts).ok()
}

/// Seconds from `ts` to now, rounded like Ruby's `(Time.now - Time.parse(ts)).round`.
pub fn seconds_since(ts: &str) -> Option<i64> {
    let then = parse(ts)?;
    let delta = Local::now().signed_duration_since(then);
    Some((delta.num_milliseconds() as f64 / 1000.0).round() as i64)
}

/// `Time.parse(ts).to_f`: seconds since the epoch, as Ruby computes them (the nanosecond count as a
/// double divided by 1e9). None when the text is not a timestamp.
pub fn parse_f64(ts: &str) -> Option<f64> {
    let t = parse(ts)?;
    let nanos = t.timestamp() as i128 * 1_000_000_000 + t.timestamp_subsec_nanos() as i128;
    Some(nanos as f64 / 1e9)
}

/// `Time.now.to_f`.
pub fn now_f64() -> f64 {
    let d = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    d.as_nanos() as f64 / 1e9
}

/// `Time#to_f` of a file time: the nanosecond count as a double divided by 1e9.
pub fn epoch_f64(t: std::time::SystemTime) -> f64 {
    match t.duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => d.as_nanos() as f64 / 1e9,
        Err(e) => -(e.duration().as_nanos() as f64 / 1e9),
    }
}
