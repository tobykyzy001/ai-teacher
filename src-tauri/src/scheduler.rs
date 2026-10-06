use chrono::{Duration, NaiveDate};

/// 前 6 次复习通过的基准间隔（天）。
pub const BASELINE: [i64; 6] = [1, 2, 4, 7, 15, 30];
pub const MIN_EASE: f64 = 1.3;
pub const INITIAL_EASE: f64 = 2.5;

/// 通过复习时 ease 的增量（对应 SM-2 主公式按质量 4 评分的目标增益）。
const PASS_EASE_DELTA: f64 = 0.02;
/// 复习失败时 ease 的减量。
const FAIL_EASE_DELTA: f64 = -0.32;

/// SM-2 混合调度：返回 (new_reps, new_ease, new_interval_days)。
pub fn next_schedule(reps: i64, ease: f64, prev_interval: i64, passed: bool) -> (i64, f64, i64) {
    if passed {
        let new_ease = (ease + PASS_EASE_DELTA).max(MIN_EASE);
        let new_reps = reps + 1;
        let interval = if new_reps <= BASELINE.len() as i64 {
            BASELINE[(new_reps - 1) as usize]
        } else {
            (prev_interval as f64 * new_ease).round() as i64
        };
        (new_reps, new_ease, interval.max(1))
    } else {
        (0, (ease + FAIL_EASE_DELTA).max(MIN_EASE), 1)
    }
}

/// 到期日 = today + interval，格式 YYYY-MM-DD。
pub fn due_date(today: NaiveDate, interval_days: i64) -> String {
    (today + Duration::days(interval_days))
        .format("%Y-%m-%d")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pass_seq(n: usize) -> (i64, f64, i64) {
        let (mut reps, mut ease, mut interval) = (0i64, 2.5f64, 1i64);
        for _ in 0..n {
            let r = next_schedule(reps, ease, interval, true);
            (reps, ease, interval) = r;
        }
        (reps, ease, interval)
    }

    #[test]
    fn baseline_intervals_for_first_six_passes() {
        let expected = [1i64, 2, 4, 7, 15, 30];
        let (mut reps, mut ease, mut interval) = (0i64, 2.5f64, 1i64);
        for (i, exp) in expected.iter().enumerate() {
            let r = next_schedule(reps, ease, interval, true);
            assert_eq!(r.2, *exp, "第 {} 次通过的间隔", i + 1);
            assert_eq!(r.0, (i + 1) as i64);
            (reps, ease, interval) = r;
        }
        assert!((ease - 2.62).abs() < 1e-9, "ease = {ease}");
    }

    #[test]
    fn after_baseline_interval_multiplies_by_ease() {
        let (reps, ease, interval) = pass_seq(6);
        assert_eq!(interval, 30);
        let (reps7, ease7, interval7) = next_schedule(reps, ease, interval, true);
        assert_eq!(reps7, 7);
        assert_eq!(interval7, 79, "round(30 * 2.62)");
        let (_, ease8, interval8) = next_schedule(reps7, ease7, interval7, true);
        assert_eq!(interval8, (interval7 as f64 * ease8).round() as i64);
        assert!(interval8 > interval7);
    }

    #[test]
    fn fail_resets_reps_interval_and_drops_ease() {
        let (reps, ease, interval) = next_schedule(3, 2.5, 4, false);
        assert_eq!(reps, 0);
        assert_eq!(interval, 1);
        assert!((ease - 2.18).abs() < 1e-9, "ease = {ease}");
    }

    #[test]
    fn ease_never_below_min() {
        let (_, ease, _) = next_schedule(2, 1.4, 4, false);
        assert!((ease - MIN_EASE).abs() < 1e-9, "ease = {ease}");
        let (_, ease, _) = next_schedule(0, 1.2, 1, false);
        assert!((ease - MIN_EASE).abs() < 1e-9, "ease = {ease}");
        let (_, ease, _) = next_schedule(0, 1.3, 1, true);
        assert!((ease - 1.32).abs() < 1e-9, "ease = {ease}");
    }

    #[test]
    fn due_date_formatting() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 6).unwrap();
        assert_eq!(due_date(today, 0), "2026-10-06");
        assert_eq!(due_date(today, 1), "2026-10-07");
        assert_eq!(due_date(today, 30), "2026-11-05");
    }
}
