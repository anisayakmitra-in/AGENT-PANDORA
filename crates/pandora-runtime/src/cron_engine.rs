use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CronJobStatus {
    Active,
    Paused,
    Completed,
}

impl CronJobStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Paused => "paused",
            Self::Completed => "completed",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CronError {
    InvalidSchedule(String),
    JobNotFound(String),
    DuplicateJobName(String),
    Storage(String),
}

impl std::fmt::Display for CronError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidSchedule(msg) => write!(f, "invalid schedule: {msg}"),
            Self::JobNotFound(name) => write!(f, "scheduled job '{name}' not found"),
            Self::DuplicateJobName(name) => write!(f, "scheduled job name '{name}' already exists"),
            Self::Storage(err) => write!(f, "cron storage error: {err}"),
        }
    }
}

impl std::error::Error for CronError {}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CronJob {
    pub id: String,
    pub name: String,
    pub schedule_expr: String,
    pub command: String,
    #[serde(default)]
    pub workdir: Option<PathBuf>,
    #[serde(default = "default_deliver")]
    pub deliver: String,
    pub next_run_at: i64,
    #[serde(default)]
    pub last_run_at: Option<i64>,
    pub status: CronJobStatus,
    #[serde(default)]
    pub runs_count: u64,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_deliver() -> String {
    "local".to_owned()
}

const fn default_true() -> bool {
    true
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CronExecutionResult {
    pub job_id: String,
    pub name: String,
    pub command: String,
    pub executed_at: i64,
    pub success: bool,
    pub output: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CronSchedule {
    Delay(u64),                       // one-shot delay in seconds
    Interval(u64),                    // recurring interval in seconds
    Daily { hour: u32, minute: u32 }, // recurring daily at HH:MM
    CronExpr(CronFieldMatcher),       // 5-field cron
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CronFieldMatcher {
    minutes: Vec<u32>,
    hours: Vec<u32>,
    days_of_month: Vec<u32>,
    months: Vec<u32>,
    days_of_week: Vec<u32>,
}

impl CronSchedule {
    pub fn parse(expr: &str, now: i64) -> Result<(Self, i64, bool), CronError> {
        let trimmed = expr.trim();
        if trimmed.is_empty() {
            return Err(CronError::InvalidSchedule(
                "empty schedule expression".into(),
            ));
        }

        let lower = trimmed.to_lowercase();

        // 1. One-shot relative delay: "in 30m", "in 2h", "in 1d", "in 10s"
        if let Some(rest) = lower.strip_prefix("in ") {
            let secs = parse_duration_seconds(rest.trim())?;
            let next_run = now.saturating_add(secs as i64);
            return Ok((Self::Delay(secs), next_run, true));
        }

        // 2. Recurring interval: "every 2h", "every 30m", "every 1d", "every 10s"
        if let Some(rest) = lower.strip_prefix("every ") {
            let rest = rest.trim();
            if rest == "day" || rest == "hour" || rest == "minute" {
                let secs = match rest {
                    "day" => 86400,
                    "hour" => 3600,
                    _ => 60,
                };
                let next_run = now.saturating_add(secs as i64);
                return Ok((Self::Interval(secs), next_run, false));
            }
            if let Some(time_str) = rest.strip_prefix("day at ") {
                let (h, m) = parse_hh_mm(time_str.trim())?;
                let next_run = next_daily_occurrence(now, h, m);
                return Ok((Self::Daily { hour: h, minute: m }, next_run, false));
            }
            if let Ok(secs) = parse_duration_seconds(rest) {
                let next_run = now.saturating_add(secs as i64);
                return Ok((Self::Interval(secs), next_run, false));
            }
        }

        // 3. Daily: "daily at 09:00" or "daily at 14:30"
        if let Some(time_str) = lower.strip_prefix("daily at ") {
            let (h, m) = parse_hh_mm(time_str.trim())?;
            let next_run = next_daily_occurrence(now, h, m);
            return Ok((Self::Daily { hour: h, minute: m }, next_run, false));
        }

        // 4. Hourly: "hourly"
        if lower == "hourly" {
            let secs = 3600;
            let next_run = now.saturating_add(secs as i64);
            return Ok((Self::Interval(secs), next_run, false));
        }

        // 5. Standard 5-field cron: "* * * * *"
        let parts: Vec<&str> = trimmed.split_whitespace().collect();
        if parts.len() == 5 {
            let matcher = parse_5_field_cron(&parts)?;
            let next_run = matcher.next_run_after(now)?;
            return Ok((Self::CronExpr(matcher), next_run, false));
        }

        Err(CronError::InvalidSchedule(format!(
            "unsupported schedule format '{expr}' (supported: 'in 30m', 'every 2h', 'daily at 09:00', or '0 9 * * *')"
        )))
    }

    pub fn compute_next_run(&self, now: i64) -> Result<Option<i64>, CronError> {
        match self {
            Self::Delay(_) => Ok(None), // one-shot, no next run
            Self::Interval(secs) => Ok(Some(now.saturating_add(*secs as i64))),
            Self::Daily { hour, minute } => Ok(Some(next_daily_occurrence(now, *hour, *minute))),
            Self::CronExpr(matcher) => matcher.next_run_after(now).map(Some),
        }
    }
}

fn parse_duration_seconds(s: &str) -> Result<u64, CronError> {
    let s = s.trim();
    let num_end = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    if num_end == 0 {
        return Err(CronError::InvalidSchedule(format!(
            "invalid duration '{s}'"
        )));
    }
    let num: u64 = s[..num_end]
        .parse()
        .map_err(|_| CronError::InvalidSchedule(format!("invalid number in duration '{s}'")))?;
    let unit = s[num_end..].trim().to_ascii_lowercase();

    let multiplier = match unit.as_str() {
        "s" | "sec" | "second" | "seconds" => 1,
        "m" | "min" | "minute" | "minutes" => 60,
        "h" | "hr" | "hour" | "hours" => 3600,
        "d" | "day" | "days" => 86400,
        "w" | "week" | "weeks" => 7 * 86400,
        other => {
            return Err(CronError::InvalidSchedule(format!(
                "unknown duration unit '{other}' in '{s}'"
            )));
        }
    };
    Ok(num.saturating_mul(multiplier))
}

fn parse_hh_mm(s: &str) -> Result<(u32, u32), CronError> {
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() != 2 {
        return Err(CronError::InvalidSchedule(format!(
            "expected HH:MM, got '{s}'"
        )));
    }
    let h: u32 = parts[0]
        .trim()
        .parse()
        .map_err(|_| CronError::InvalidSchedule(format!("invalid hour in '{s}'")))?;
    let m: u32 = parts[1]
        .trim()
        .parse()
        .map_err(|_| CronError::InvalidSchedule(format!("invalid minute in '{s}'")))?;
    if h > 23 || m > 59 {
        return Err(CronError::InvalidSchedule(format!(
            "hour (0-23) or minute (0-59) out of range in '{s}'"
        )));
    }
    Ok((h, m))
}

fn next_daily_occurrence(now: i64, hour: u32, minute: u32) -> i64 {
    let (_, _, _, curr_h, curr_m, _) = utc_datetime_from_timestamp(now);
    let today_target_secs = (hour * 3600 + minute * 60) as i64;
    let curr_secs_of_day = (curr_h * 3600 + curr_m * 60) as i64 + (now % 60);

    let days_offset = if today_target_secs > curr_secs_of_day {
        0
    } else {
        1
    };

    let start_of_today = now - (now % 86400);
    start_of_today + (days_offset * 86400) + today_target_secs
}

fn parse_5_field_cron(parts: &[&str]) -> Result<CronFieldMatcher, CronError> {
    if parts.len() != 5 {
        return Err(CronError::InvalidSchedule(
            "5 fields required for cron".into(),
        ));
    }
    let minutes = parse_cron_field(parts[0], 0, 59)?;
    let hours = parse_cron_field(parts[1], 0, 23)?;
    let days_of_month = parse_cron_field(parts[2], 1, 31)?;
    let months = parse_cron_field(parts[3], 1, 12)?;
    let days_of_week = parse_cron_field(parts[4], 0, 7)?; // 0 and 7 both mean Sunday
    Ok(CronFieldMatcher {
        minutes,
        hours,
        days_of_month,
        months,
        days_of_week,
    })
}

fn parse_cron_field(field: &str, min: u32, max: u32) -> Result<Vec<u32>, CronError> {
    let field = field.trim();
    if field == "*" {
        return Ok((min..=max).collect());
    }

    if let Some(step_str) = field.strip_prefix("*/") {
        let step: u32 = step_str
            .parse()
            .map_err(|_| CronError::InvalidSchedule(format!("invalid step in '{field}'")))?;
        if step == 0 {
            return Err(CronError::InvalidSchedule("cron step cannot be 0".into()));
        }
        return Ok((min..=max).step_by(step as usize).collect());
    }

    let mut result = Vec::new();
    for part in field.split(',') {
        let part = part.trim();
        if let Some((start_s, end_s)) = part.split_once('-') {
            let start: u32 = start_s.parse().map_err(|_| {
                CronError::InvalidSchedule(format!("invalid range start in '{part}'"))
            })?;
            let end: u32 = end_s.parse().map_err(|_| {
                CronError::InvalidSchedule(format!("invalid range end in '{part}'"))
            })?;
            if start < min || end > max || start > end {
                return Err(CronError::InvalidSchedule(format!(
                    "range {start}-{end} out of bounds [{min}-{max}] in '{part}'"
                )));
            }
            result.extend(start..=end);
        } else {
            let val: u32 = part
                .parse()
                .map_err(|_| CronError::InvalidSchedule(format!("invalid cron value '{part}'")))?;
            if val < min || val > max {
                return Err(CronError::InvalidSchedule(format!(
                    "value {val} out of bounds [{min}-{max}] in '{part}'"
                )));
            }
            result.push(val);
        }
    }
    result.sort_unstable();
    result.dedup();
    Ok(result)
}

impl CronFieldMatcher {
    pub fn next_run_after(&self, now: i64) -> Result<i64, CronError> {
        // Advance to next whole minute
        let mut candidate = (now / 60 + 1) * 60;
        // Search up to 366 days ahead (527,040 minutes)
        let max_search = candidate + 366 * 86400;

        while candidate < max_search {
            let (year, month, day, hour, minute, dow) = utc_datetime_from_timestamp(candidate);
            let _ = year;

            if !self.months.contains(&month) {
                // Advance to start of next month
                candidate = advance_to_next_month(candidate);
                continue;
            }

            let dow_match = self.days_of_week.contains(&dow)
                || (dow == 0 && self.days_of_week.contains(&7))
                || (dow == 7 && self.days_of_week.contains(&0));

            let dom_match = self.days_of_month.contains(&day);

            if !dom_match || !dow_match {
                // Advance to start of next day
                candidate = (candidate / 86400 + 1) * 86400;
                continue;
            }

            if !self.hours.contains(&hour) {
                // Advance to start of next hour
                candidate = (candidate / 3600 + 1) * 3600;
                continue;
            }

            if self.minutes.contains(&minute) {
                return Ok(candidate);
            }

            candidate += 60;
        }

        Err(CronError::InvalidSchedule(
            "no matching cron time found within 366 days".into(),
        ))
    }
}

fn advance_to_next_month(candidate: i64) -> i64 {
    let (y, m, _, _, _, _) = utc_datetime_from_timestamp(candidate);
    let (next_y, next_m) = if m == 12 { (y + 1, 1) } else { (y, m + 1) };
    let days = days_from_civil(next_y, next_m, 1);
    days * 86400
}

fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1024 + doe / 1461 - doe / 142401) / 365;
    let y = yoe as i32 + era as i32 * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

fn days_from_civil(y: i32, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = (if y >= 0 { y } else { y - 399 } / 400) as i64;
    let yoe = (y as i64 - era * 400) as u32;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + (doe as i64) - 719468
}

fn utc_datetime_from_timestamp(timestamp: i64) -> (i32, u32, u32, u32, u32, u32) {
    let days = timestamp.div_euclid(86400);
    let rem_secs = timestamp.rem_euclid(86400);
    let hour = (rem_secs / 3600) as u32;
    let minute = ((rem_secs % 3600) / 60) as u32;
    let (y, m, d) = civil_from_days(days);
    let dow = ((days + 4).rem_euclid(7)) as u32; // 0 = Sunday, 1 = Monday, ..., 6 = Saturday
    (y, m, d, hour, minute, dow)
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct JobsDatabase {
    jobs: Vec<CronJob>,
}

pub struct CronEngine {
    storage_path: PathBuf,
}

impl CronEngine {
    pub fn open(storage_path: impl Into<PathBuf>) -> Result<Self, CronError> {
        let storage_path = storage_path.into();
        if let Some(parent) = storage_path.parent() {
            fs::create_dir_all(parent).map_err(|e| CronError::Storage(e.to_string()))?;
        }
        Ok(Self { storage_path })
    }

    fn load(&self) -> Result<JobsDatabase, CronError> {
        if !self.storage_path.exists() {
            return Ok(JobsDatabase::default());
        }
        let data = fs::read(&self.storage_path).map_err(|e| CronError::Storage(e.to_string()))?;
        serde_json::from_slice(&data).map_err(|e| CronError::Storage(e.to_string()))
    }

    fn save(&self, db: &JobsDatabase) -> Result<(), CronError> {
        if let Some(parent) = self.storage_path.parent() {
            fs::create_dir_all(parent).map_err(|e| CronError::Storage(e.to_string()))?;
        }
        let data = serde_json::to_vec_pretty(db).map_err(|e| CronError::Storage(e.to_string()))?;
        fs::write(&self.storage_path, data).map_err(|e| CronError::Storage(e.to_string()))
    }

    pub fn list_jobs(&self) -> Result<Vec<CronJob>, CronError> {
        let db = self.load()?;
        Ok(db.jobs)
    }

    pub fn get_job(&self, id_or_name: &str) -> Result<Option<CronJob>, CronError> {
        let db = self.load()?;
        Ok(db
            .jobs
            .into_iter()
            .find(|j| j.id == id_or_name || j.name.eq_ignore_ascii_case(id_or_name)))
    }

    pub fn add_job(
        &self,
        name: &str,
        schedule_expr: &str,
        command: &str,
        workdir: Option<PathBuf>,
        deliver: Option<String>,
        now: i64,
    ) -> Result<CronJob, CronError> {
        let mut db = self.load()?;
        let name = name.trim();
        if name.is_empty() {
            return Err(CronError::InvalidSchedule(
                "job name cannot be empty".into(),
            ));
        }
        if db.jobs.iter().any(|j| j.name.eq_ignore_ascii_case(name)) {
            return Err(CronError::DuplicateJobName(name.to_owned()));
        }

        let (_parsed_sched, next_run_at, _) = CronSchedule::parse(schedule_expr, now)?;
        let id = format!(
            "cron-{}",
            pandora_types::hash_artifact(
                format!("{name}:{schedule_expr}:{command}:{now}").as_bytes()
            )
            .chars()
            .take(12)
            .collect::<String>()
        );

        let job = CronJob {
            id,
            name: name.to_owned(),
            schedule_expr: schedule_expr.to_owned(),
            command: command.to_owned(),
            workdir,
            deliver: deliver.unwrap_or_else(default_deliver),
            next_run_at,
            last_run_at: None,
            status: CronJobStatus::Active,
            runs_count: 0,
            enabled: true,
        };

        db.jobs.push(job.clone());
        self.save(&db)?;
        Ok(job)
    }

    pub fn remove_job(&self, id_or_name: &str) -> Result<bool, CronError> {
        let mut db = self.load()?;
        let initial_len = db.jobs.len();
        db.jobs
            .retain(|j| j.id != id_or_name && !j.name.eq_ignore_ascii_case(id_or_name));
        let removed = db.jobs.len() < initial_len;
        if removed {
            self.save(&db)?;
        }
        Ok(removed)
    }

    pub fn pause_job(&self, id_or_name: &str) -> Result<CronJob, CronError> {
        let mut db = self.load()?;
        let job = db
            .jobs
            .iter_mut()
            .find(|j| j.id == id_or_name || j.name.eq_ignore_ascii_case(id_or_name))
            .ok_or_else(|| CronError::JobNotFound(id_or_name.to_owned()))?;
        job.status = CronJobStatus::Paused;
        job.enabled = false;
        let updated = job.clone();
        self.save(&db)?;
        Ok(updated)
    }

    pub fn resume_job(&self, id_or_name: &str, now: i64) -> Result<CronJob, CronError> {
        let mut db = self.load()?;
        let job = db
            .jobs
            .iter_mut()
            .find(|j| j.id == id_or_name || j.name.eq_ignore_ascii_case(id_or_name))
            .ok_or_else(|| CronError::JobNotFound(id_or_name.to_owned()))?;
        let (_sched, next_run, _) = CronSchedule::parse(&job.schedule_expr, now)?;
        job.status = CronJobStatus::Active;
        job.enabled = true;
        job.next_run_at = next_run;
        let updated = job.clone();
        self.save(&db)?;
        Ok(updated)
    }

    pub fn update_job(
        &self,
        id_or_name: &str,
        new_schedule: Option<&str>,
        new_command: Option<&str>,
        now: i64,
    ) -> Result<CronJob, CronError> {
        let mut db = self.load()?;
        let job = db
            .jobs
            .iter_mut()
            .find(|j| j.id == id_or_name || j.name.eq_ignore_ascii_case(id_or_name))
            .ok_or_else(|| CronError::JobNotFound(id_or_name.to_owned()))?;

        if let Some(cmd) = new_command {
            job.command = cmd.to_owned();
        }
        if let Some(sched) = new_schedule {
            let (_sched, next_run, _) = CronSchedule::parse(sched, now)?;
            job.schedule_expr = sched.to_owned();
            job.next_run_at = next_run;
        }
        let updated = job.clone();
        self.save(&db)?;
        Ok(updated)
    }

    pub fn run_job(
        &self,
        id_or_name: &str,
        now: i64,
    ) -> Result<(CronJob, CronExecutionResult), CronError> {
        let mut db = self.load()?;
        let job = db
            .jobs
            .iter_mut()
            .find(|j| j.id == id_or_name || j.name.eq_ignore_ascii_case(id_or_name))
            .ok_or_else(|| CronError::JobNotFound(id_or_name.to_owned()))?;

        let (sched, _, is_one_shot) = CronSchedule::parse(&job.schedule_expr, now)?;
        job.last_run_at = Some(now);
        job.runs_count = job.runs_count.saturating_add(1);

        if is_one_shot {
            job.status = CronJobStatus::Completed;
            job.enabled = false;
        } else if let Ok(Some(next)) = sched.compute_next_run(now) {
            job.next_run_at = next;
        }

        let updated_job = job.clone();
        let execution = CronExecutionResult {
            job_id: updated_job.id.clone(),
            name: updated_job.name.clone(),
            command: updated_job.command.clone(),
            executed_at: now,
            success: true,
            output: format!("Executed scheduled command: {}", updated_job.command),
        };

        self.save(&db)?;
        Ok((updated_job, execution))
    }

    pub fn run_due(&self, now: i64) -> Result<Vec<(CronJob, CronExecutionResult)>, CronError> {
        let mut db = self.load()?;
        let mut executed = Vec::new();

        for job in db.jobs.iter_mut() {
            if job.status == CronJobStatus::Active && job.enabled && job.next_run_at <= now {
                let (sched, _, is_one_shot) = CronSchedule::parse(&job.schedule_expr, now)?;
                job.last_run_at = Some(now);
                job.runs_count = job.runs_count.saturating_add(1);

                if is_one_shot {
                    job.status = CronJobStatus::Completed;
                    job.enabled = false;
                } else if let Ok(Some(next)) = sched.compute_next_run(now) {
                    job.next_run_at = next;
                }

                executed.push((
                    job.clone(),
                    CronExecutionResult {
                        job_id: job.id.clone(),
                        name: job.name.clone(),
                        command: job.command.clone(),
                        executed_at: now,
                        success: true,
                        output: format!("Executed scheduled command: {}", job.command),
                    },
                ));
            }
        }

        if !executed.is_empty() {
            self.save(&db)?;
        }
        Ok(executed)
    }
}

pub fn handle_cronjob_manage_action(
    storage_path: impl AsRef<Path>,
    arguments: &serde_json::Value,
    now: i64,
) -> Result<serde_json::Value, String> {
    let engine = CronEngine::open(storage_path.as_ref()).map_err(|e| e.to_string())?;
    let action = arguments
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "missing required 'action'".to_string())?;

    match action {
        "list" => {
            let jobs = engine.list_jobs().map_err(|e| e.to_string())?;
            Ok(serde_json::json!({
                "status": "success",
                "jobs": jobs,
                "count": jobs.len()
            }))
        }
        "create" => {
            let schedule = arguments
                .get("schedule")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "action 'create' requires 'schedule'".to_string())?;
            let command = arguments
                .get("prompt")
                .or_else(|| arguments.get("command"))
                .and_then(|v| v.as_str())
                .ok_or_else(|| "action 'create' requires 'prompt' or 'command'".to_string())?;
            let name = arguments
                .get("name")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| {
                    format!(
                        "job-{}",
                        pandora_types::hash_artifact(
                            format!("{schedule}:{command}:{now}").as_bytes()
                        )
                        .chars()
                        .take(8)
                        .collect::<String>()
                    )
                });
            let workdir = arguments
                .get("workdir")
                .and_then(|v| v.as_str())
                .map(PathBuf::from);
            let deliver = arguments
                .get("deliver")
                .and_then(|v| v.as_str())
                .map(String::from);

            let job = engine
                .add_job(&name, schedule, command, workdir, deliver, now)
                .map_err(|e| e.to_string())?;
            Ok(serde_json::json!({
                "status": "created",
                "job": job
            }))
        }
        "pause" => {
            let job_id = arguments
                .get("job_id")
                .or_else(|| arguments.get("name"))
                .and_then(|v| v.as_str())
                .ok_or_else(|| "action 'pause' requires 'job_id' or 'name'".to_string())?;
            let job = engine.pause_job(job_id).map_err(|e| e.to_string())?;
            Ok(serde_json::json!({
                "status": "paused",
                "job": job
            }))
        }
        "resume" => {
            let job_id = arguments
                .get("job_id")
                .or_else(|| arguments.get("name"))
                .and_then(|v| v.as_str())
                .ok_or_else(|| "action 'resume' requires 'job_id' or 'name'".to_string())?;
            let job = engine.resume_job(job_id, now).map_err(|e| e.to_string())?;
            Ok(serde_json::json!({
                "status": "resumed",
                "job": job
            }))
        }
        "update" => {
            let job_id = arguments
                .get("job_id")
                .or_else(|| arguments.get("name"))
                .and_then(|v| v.as_str())
                .ok_or_else(|| "action 'update' requires 'job_id' or 'name'".to_string())?;
            let schedule = arguments.get("schedule").and_then(|v| v.as_str());
            let command = arguments
                .get("prompt")
                .or_else(|| arguments.get("command"))
                .and_then(|v| v.as_str());
            let job = engine
                .update_job(job_id, schedule, command, now)
                .map_err(|e| e.to_string())?;
            Ok(serde_json::json!({
                "status": "updated",
                "job": job
            }))
        }
        "remove" => {
            let job_id = arguments
                .get("job_id")
                .or_else(|| arguments.get("name"))
                .and_then(|v| v.as_str())
                .ok_or_else(|| "action 'remove' requires 'job_id' or 'name'".to_string())?;
            let removed = engine.remove_job(job_id).map_err(|e| e.to_string())?;
            Ok(serde_json::json!({
                "status": if removed { "removed" } else { "not_found" },
                "job_id": job_id
            }))
        }
        "run" => {
            let job_id = arguments
                .get("job_id")
                .or_else(|| arguments.get("name"))
                .and_then(|v| v.as_str())
                .ok_or_else(|| "action 'run' requires 'job_id' or 'name'".to_string())?;
            let (job, execution) = engine.run_job(job_id, now).map_err(|e| e.to_string())?;
            Ok(serde_json::json!({
                "status": "executed",
                "job": job,
                "execution": execution
            }))
        }
        unknown => Err(format!(
            "unknown action '{unknown}'; expected create, list, pause, resume, update, remove, or run"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_relative_delay_schedule() {
        let now = 1700000000;
        let (sched, next_run, is_one_shot) = CronSchedule::parse("in 30m", now).unwrap();
        assert!(is_one_shot);
        assert_eq!(next_run, now + 1800);
        assert_eq!(sched, CronSchedule::Delay(1800));
        assert_eq!(sched.compute_next_run(next_run).unwrap(), None);
    }

    #[test]
    fn test_interval_schedule() {
        let now = 1700000000;
        let (sched, next_run, is_one_shot) = CronSchedule::parse("every 2h", now).unwrap();
        assert!(!is_one_shot);
        assert_eq!(next_run, now + 7200);
        assert_eq!(sched, CronSchedule::Interval(7200));
        assert_eq!(
            sched.compute_next_run(next_run).unwrap(),
            Some(next_run + 7200)
        );
    }

    #[test]
    fn test_daily_schedule() {
        let now = 1700000000; // Thursday 2023-11-14 ~22:13 UTC
        let (sched, next_run, is_one_shot) = CronSchedule::parse("daily at 09:00", now).unwrap();
        assert!(!is_one_shot);
        assert!(next_run > now);
        match sched {
            CronSchedule::Daily { hour, minute } => {
                assert_eq!(hour, 9);
                assert_eq!(minute, 0);
            }
            _ => panic!("expected daily schedule"),
        }
    }

    #[test]
    fn test_cron_5_field_schedule() {
        let now = 1700000000;
        let (_sched, next_run, is_one_shot) = CronSchedule::parse("0 9 * * *", now).unwrap();
        assert!(!is_one_shot);
        assert!(next_run > now);
        let (_, _, _, h, m, _) = utc_datetime_from_timestamp(next_run);
        assert_eq!(h, 9);
        assert_eq!(m, 0);
    }

    #[test]
    fn test_cron_engine_lifecycle() {
        let temp_dir = std::env::temp_dir().join(format!("pandora_cron_test_{}", now_nanos()));
        let jobs_path = temp_dir.join("cron").join("jobs.json");
        let engine = CronEngine::open(&jobs_path).unwrap();

        let now = 1700000000;
        let job = engine
            .add_job(
                "HealthCheck",
                "every 1h",
                "pandora doctor",
                None,
                Some("local".into()),
                now,
            )
            .unwrap();

        assert_eq!(job.name, "HealthCheck");
        assert_eq!(job.status, CronJobStatus::Active);
        assert!(job.enabled);
        assert_eq!(job.next_run_at, now + 3600);

        let list = engine.list_jobs().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, job.id);

        // Pause
        let paused = engine.pause_job(&job.id).unwrap();
        assert_eq!(paused.status, CronJobStatus::Paused);
        assert!(!paused.enabled);

        // Resume
        let resumed = engine.resume_job(&job.name, now).unwrap();
        assert_eq!(resumed.status, CronJobStatus::Active);
        assert!(resumed.enabled);

        // Run due (when now < next_run_at -> none due)
        let due = engine.run_due(now).unwrap();
        assert!(due.is_empty());

        // Run due (when now >= next_run_at)
        let due = engine.run_due(now + 4000).unwrap();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].0.runs_count, 1);
        assert_eq!(due[0].0.last_run_at, Some(now + 4000));

        // Remove
        let removed = engine.remove_job(&job.id).unwrap();
        assert!(removed);
        assert!(engine.list_jobs().unwrap().is_empty());

        let _ = fs::remove_dir_all(&temp_dir);
    }

    fn now_nanos() -> u128 {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    }
}
