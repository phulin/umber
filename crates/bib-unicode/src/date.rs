#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DateError {
    Empty,
    InvalidFormat,
    InvalidMonth,
    InvalidDay,
    InvalidTime,
    ReversedRange,
    TooLong,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Uncertainty {
    Certain,
    Uncertain,
    Approximate,
    UncertainApproximate,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum YearDivision {
    Spring,
    Summer,
    Autumn,
    Winter,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DatePart {
    pub year: String,
    pub month: Option<String>,
    pub day: Option<String>,
    pub division: Option<YearDivision>,
    pub uncertainty: Uncertainty,
    pub open: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DateTime {
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
    pub millisecond: u16,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExtendedDate {
    pub start: Option<DatePart>,
    pub end: Option<DatePart>,
    pub time: Option<DateTime>,
}

impl ExtendedDate {
    pub fn parse(input: &str) -> Result<Self, DateError> {
        if input.is_empty() {
            return Err(DateError::Empty);
        }
        if input.len() > 256 {
            return Err(DateError::TooLong);
        }
        let (date, time) = input
            .split_once('T')
            .map_or((input, None), |(d, t)| (d, Some(parse_time(t))));
        let time = time.transpose()?;
        let (start, end) = if let Some((a, b)) = date.split_once('/') {
            (parse_optional(a)?, parse_optional(b)?)
        } else {
            (parse_optional(date)?, None)
        };
        Ok(Self { start, end, time })
    }
}

fn parse_optional(value: &str) -> Result<Option<DatePart>, DateError> {
    if value.is_empty() || value == ".." {
        return Ok(None);
    }
    let mut raw = value;
    let uncertainty = if let Some(v) = raw.strip_suffix("%") {
        raw = v;
        Uncertainty::UncertainApproximate
    } else if let Some(v) = raw.strip_suffix('?') {
        raw = v;
        Uncertainty::Uncertain
    } else if let Some(v) = raw.strip_suffix('~') {
        raw = v;
        Uncertainty::Approximate
    } else {
        Uncertainty::Certain
    };
    let open = raw.ends_with("XX") || raw.ends_with("xx");
    let pieces: Vec<_> = raw.split('-').collect();
    let (year_index, negative) = if pieces.first() == Some(&"") {
        (1, true)
    } else {
        (0, false)
    };
    let year_raw = pieces.get(year_index).ok_or(DateError::InvalidFormat)?;
    if year_raw.is_empty()
        || !year_raw
            .chars()
            .all(|c| c.is_numeric() || c == 'X' || c == 'x')
    {
        return Err(DateError::InvalidFormat);
    }
    let year = if negative {
        format!("-{year_raw}")
    } else {
        (*year_raw).to_owned()
    };
    let month_raw = pieces.get(year_index + 1).copied();
    let day_raw = pieces.get(year_index + 2).copied();
    if pieces.len() > year_index + 3 {
        return Err(DateError::InvalidFormat);
    }
    let mut division = None;
    if let Some(m) = month_raw {
        if m.chars().count() != 2 {
            return Err(DateError::InvalidFormat);
        }
        if let Ok(number) = m.parse::<u8>() {
            if (21..=24).contains(&number) {
                division = Some(match number {
                    21 => YearDivision::Spring,
                    22 => YearDivision::Summer,
                    23 => YearDivision::Autumn,
                    _ => YearDivision::Winter,
                });
            } else if !(1..=12).contains(&number) {
                return Err(DateError::InvalidMonth);
            }
        } else if !m.chars().all(|c| c == 'X' || c == 'x' || c.is_numeric()) {
            return Err(DateError::InvalidMonth);
        }
    }
    if let Some(d) = day_raw {
        if d.chars().count() != 2 {
            return Err(DateError::InvalidFormat);
        }
        if let Ok(number) = d.parse::<u8>() {
            if !(1..=31).contains(&number) {
                return Err(DateError::InvalidDay);
            }
        } else if !d.chars().all(|c| c == 'X' || c == 'x' || c.is_numeric()) {
            return Err(DateError::InvalidDay);
        }
    }
    Ok(Some(DatePart {
        year,
        month: month_raw.map(str::to_owned),
        day: day_raw.map(str::to_owned),
        division,
        uncertainty,
        open,
    }))
}

fn parse_time(value: &str) -> Result<DateTime, DateError> {
    let value = value.strip_suffix('Z').unwrap_or(value);
    let mut parts = value.split(':');
    let hour = parts
        .next()
        .and_then(|v| v.parse().ok())
        .ok_or(DateError::InvalidTime)?;
    let minute = parts
        .next()
        .and_then(|v| v.parse().ok())
        .ok_or(DateError::InvalidTime)?;
    let seconds = parts.next().ok_or(DateError::InvalidTime)?;
    if parts.next().is_some() || hour > 23 || minute > 59 {
        return Err(DateError::InvalidTime);
    }
    let (second, millisecond) = if let Some((second, milliseconds)) = seconds.split_once('.') {
        let padded = format!("{milliseconds:0<3}");
        (
            second.parse().map_err(|_| DateError::InvalidTime)?,
            padded[..3].parse().map_err(|_| DateError::InvalidTime)?,
        )
    } else {
        (seconds.parse().map_err(|_| DateError::InvalidTime)?, 0)
    };
    if second > 59 {
        return Err(DateError::InvalidTime);
    }
    Ok(DateTime {
        hour,
        minute,
        second,
        millisecond,
    })
}
