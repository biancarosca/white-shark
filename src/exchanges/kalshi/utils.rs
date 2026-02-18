use std::time::Duration;

use chrono::{Datelike, Timelike, TimeZone, Utc, Weekday};
use chrono_tz::US::Eastern;
use tokio::time::Instant as TokioInstant;

use crate::exchanges::kalshi::constants::FETCH_AFTER_CLOSE_SECS;

pub fn maintenance_sleep_duration() -> Option<Duration> {
    let now_et = Utc::now().with_timezone(&Eastern);
    if now_et.weekday() == Weekday::Thu && now_et.hour() >= 3 && now_et.hour() < 5 {
        let end = now_et.date_naive().and_hms_opt(5, 0, 0).unwrap();
        let end_utc = Eastern.from_local_datetime(&end).unwrap().with_timezone(&Utc);
        let remaining = (end_utc - Utc::now()).to_std().unwrap_or(Duration::from_secs(60));
        Some(remaining)
    } else {
        None
    }
}

pub fn next_maintenance_start() -> TokioInstant {
    let now = Utc::now();
    let now_et = now.with_timezone(&Eastern);

    let mut target = now_et.date_naive();
    loop {
        if target.weekday() == Weekday::Thu {
            let start = target.and_hms_opt(3, 0, 0).unwrap();
            if let Some(start_et) = Eastern.from_local_datetime(&start).single() {
                let start_utc = start_et.with_timezone(&Utc);
                if start_utc > now {
                    let secs = (start_utc - now).num_seconds().max(0) as u64;
                    return TokioInstant::now() + Duration::from_secs(secs);
                }
            }
        }
        target += chrono::Duration::days(1);
    }
}

pub fn next_15min_interval() -> TokioInstant {
    let now = Utc::now();
    let seconds_since_hour = now.timestamp() % 3600;
    let seconds_into_15min_block = seconds_since_hour % 900;

    let seconds_until_next_15min = if seconds_into_15min_block == 0 {
        FETCH_AFTER_CLOSE_SECS as u64
    } else {
        (900 - seconds_into_15min_block) as u64 + FETCH_AFTER_CLOSE_SECS as u64
    };

    TokioInstant::now() + Duration::from_secs(seconds_until_next_15min)
}