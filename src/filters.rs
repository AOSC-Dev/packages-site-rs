use crate::utils::ver_rel;
use std::fmt::Display;

macro_rules! bail {
    ($($arg:tt)*) => {
        return Err(askama::Error::Custom(anyhow::anyhow!(format!($($arg)*)).into()))
    };
}

#[askama::filter_fn]
pub fn d(s: &str, _: &dyn askama::Values, default: &str, consider_empty: bool) -> ::askama::Result<String> {
    let _ = consider_empty;
    if !s.is_empty() {
        Ok(s.to_string())
    } else {
        Ok(default.to_string())
    }
}

#[askama::filter_fn]
pub fn fmt_timestamp(timestamp: &time::OffsetDateTime, _: &dyn askama::Values) -> ::askama::Result<String> {
    if let Ok(date) = timestamp.format(&time::format_description::well_known::Rfc2822) {
        return Ok(date);
    }
    bail!("cannot format timestamp {timestamp} into RFC2822 format")
}

#[askama::filter_fn]
pub fn cut(s: &str, _: &dyn askama::Values, len: usize) -> ::askama::Result<String> {
    if s.len() <= len {
        Ok(s.to_string())
    } else {
        Ok(s[..len].to_string())
    }
}

#[askama::filter_fn]
pub fn fill<S: AsRef<str>>(
    s: S,
    _: &dyn askama::Values,
    width: usize,
    subsequent_indent: &str,
) -> ::askama::Result<String> {
    let opt = textwrap::Options::new(width).subsequent_indent(subsequent_indent);
    Ok(textwrap::fill(s.as_ref(), opt))
}

#[askama::filter_fn]
pub fn get_first_line(s: &str, _: &dyn askama::Values) -> ::askama::Result<String> {
    Ok(s.lines().next().unwrap_or("").to_string())
}

fn strftime_impl(datetime: &time::OffsetDateTime, s: &str) -> ::askama::Result<String> {
    match time::format_description::parse_borrowed::<2>(s) {
        Ok(fmt) => match datetime.format(&fmt) {
            Ok(res) => Ok(res),
            Err(e) => bail!("{}", e.to_string()),
        },
        Err(e) => bail!("{}", e.to_string()),
    }
}

#[askama::filter_fn]
pub fn strftime(datetime: &time::OffsetDateTime, _: &dyn askama::Values, s: &str) -> ::askama::Result<String> {
    strftime_impl(datetime, s)
}

#[askama::filter_fn]
pub fn strftime_i32(timestamp: &i32, _: &dyn askama::Values, s: &str) -> ::askama::Result<String> {
    match time::OffsetDateTime::from_unix_timestamp(*timestamp as i64) {
        Ok(datetime) => strftime_impl(&datetime, s),
        Err(e) => bail!("{}", e.to_string()),
    }
}

#[askama::filter_fn]
pub fn sizeof_fmt(size: &i64, _: &dyn askama::Values) -> ::askama::Result<String> {
    let size = size::Size::from_bytes(*size);
    Ok(size.to_string())
}

#[askama::filter_fn]
pub fn fmt_ver_compare(ver_compare: &i32, _: &dyn askama::Values) -> ::askama::Result<&'static str> {
    Ok(ver_rel(*ver_compare))
}

#[askama::filter_fn]
pub fn fmt_pkg_status(status: &i32, _: &dyn askama::Values) -> ::askama::Result<&'static str> {
    Ok(match *status {
        0 => "normal",
        2 => "testing",
        _ => "unknown",
    })
}

#[askama::filter_fn]
pub fn sizeof_fmt_ls(num: &i64, _: &dyn askama::Values) -> ::askama::Result<String> {
    if num.abs() < 1024 {
        return Ok(num.to_string());
    }

    let mut num = (*num as f64) / 1024.0;

    for unit in "KMGTPEZ".chars() {
        if num.abs() < 10.0 {
            return Ok(format!("{num:.1}{unit}"));
        } else if num.abs() < 1024.0 {
            return Ok(format!("{num:.0}{unit}"));
        }
        num /= 1024.0
    }

    Ok(format!("{num:.1}Y"))
}

#[askama::filter_fn]
pub fn ls_perm(perm: &i32, _: &dyn askama::Values, ftype: &i16) -> ::askama::Result<String> {
    // see https://docs.rs/tar/latest/src/tar/entry_type.rs.html#70-87
    let ftype = match ftype {
        1 => 'l',
        3 => 'c',
        4 => 'b',
        5 => 'd',
        6 => 'p',
        _ => '-',
    };

    let perm: String = format!("{perm:b}")
        .chars()
        .zip("rwxrwxrwx".chars())
        .map(|(a, b)| if a == '1' { b } else { '-' })
        .collect();

    Ok(format!("{ftype}{perm}"))
}

#[askama::filter_fn]
pub fn fmt_default<T: Display + Default>(x: &Option<T>, _: &dyn askama::Values) -> ::askama::Result<String> {
    if let Some(x) = x {
        Ok(format!("{x}"))
    } else {
        Ok(format!("{}", T::default()))
    }
}
