use gpui_component::calendar::Date as PickerDate;
use time::{Date, OffsetDateTime, PrimitiveDateTime, Time, UtcOffset, macros::format_description};

use crate::domain::Due;

pub(super) fn parse_due(value: &str) -> Result<Due, String> {
    let value = value.trim();
    if value.is_empty() || value == "未定" {
        return Ok(Due::None);
    }
    let date_format = format_description!("[year]-[month]-[day]");
    if let Ok(date) = Date::parse(value, date_format) {
        return Ok(Due::Date(date));
    }
    let date_time_format = format_description!("[year]-[month]-[day] [hour]:[minute]");
    if let Ok(date_time) = PrimitiveDateTime::parse(value, date_time_format) {
        let offset = UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC);
        return Ok(Due::DateTime(date_time.assume_offset(offset)));
    }
    Err("納期は YYYY-MM-DD または YYYY-MM-DD HH:MM で入力してください".to_owned())
}

pub(super) fn format_due_input(due: &Due) -> String {
    match due {
        Due::None => String::new(),
        Due::Date(date) => date.to_string(),
        Due::DateTime(date_time) => {
            let offset = UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC);
            date_time
                .to_offset(offset)
                .format(format_description!("[year]-[month]-[day] [hour]:[minute]"))
                .unwrap_or_default()
        }
    }
}

pub(super) fn format_due_display(due: &Due) -> String {
    match due {
        Due::None => String::new(),
        Due::Date(date) => format!("期限 {date}"),
        Due::DateTime(_) => format!("期限 {}", format_due_input(due)),
    }
}

pub(super) fn due_is_today(due: &Due, today: Date, offset: UtcOffset) -> bool {
    match due {
        Due::None => false,
        Due::Date(date) => *date == today,
        Due::DateTime(date_time) => date_time.to_offset(offset).date() == today,
    }
}

pub(super) fn due_date(due: &Due) -> Option<Date> {
    match due {
        Due::None => None,
        Due::Date(date) => Some(*date),
        Due::DateTime(date_time) => Some(
            date_time
                .to_offset(UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC))
                .date(),
        ),
    }
}

pub(super) fn picker_date_from_due(due: &Due) -> PickerDate {
    let Some(date) = due_date(due) else {
        return PickerDate::Single(None);
    };
    chrono::NaiveDate::from_ymd_opt(
        date.year(),
        u32::from(date.month() as u8),
        u32::from(date.day()),
    )
    .map(PickerDate::from)
    .unwrap_or(PickerDate::Single(None))
}

pub(super) fn picker_due_input_value(date: PickerDate, current: &str) -> String {
    let Some(date) = date.start() else {
        return String::new();
    };
    let date = date.format("%Y-%m-%d").to_string();
    match parse_due(current).ok().as_ref().and_then(due_time_value) {
        Some(time) => format!("{date} {time}"),
        None => date,
    }
}

pub(super) fn due_time_options() -> Vec<String> {
    (0..24)
        .flat_map(|hour| {
            [0, 15, 30, 45]
                .into_iter()
                .map(move |minute| format!("{hour:02}:{minute:02}"))
        })
        .collect()
}

pub(super) fn due_time_value(due: &Due) -> Option<String> {
    let Due::DateTime(date_time) = due else {
        return None;
    };
    let local = date_time.to_offset(UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC));
    Some(format!("{:02}:{:02}", local.hour(), local.minute()))
}

pub(super) fn due_input_with_time(
    current: &str,
    selected_time: Option<&str>,
) -> Result<String, String> {
    let due = parse_due(current)?;
    let Some(selected_time) = selected_time else {
        return Ok(due_date(&due).map_or_else(String::new, |date| date.to_string()));
    };
    let time_format = format_description!("[hour]:[minute]");
    let time = Time::parse(selected_time, time_format)
        .map_err(|_| "時刻は HH:MM 形式で指定してください".to_owned())?;
    let Some(date) = due_date(&due) else {
        return Err("時刻を選ぶ前に日付を指定してください".to_owned());
    };
    Ok(format!("{date} {:02}:{:02}", time.hour(), time.minute()))
}

pub(super) fn date_to_filter_datetime(date: Date) -> OffsetDateTime {
    date.with_hms(0, 0, 0)
        .expect("midnight must be valid")
        .assume_offset(UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC))
}

pub(super) fn calendar_leading_days(first: Date) -> usize {
    first.weekday().number_days_from_sunday() as usize
}

pub(super) fn shift_month(date: Date, delta: i32) -> Date {
    let mut year = date.year();
    let mut month = i32::from(date.month() as u8) + delta;
    while month < 1 {
        month += 12;
        year -= 1;
    }
    while month > 12 {
        month -= 12;
        year += 1;
    }
    let month = time::Month::try_from(month as u8).expect("month must be valid");
    Date::from_calendar_date(year, month, 1).expect("first day of month must be valid")
}
