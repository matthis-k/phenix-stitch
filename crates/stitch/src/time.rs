use chrono::{SecondsFormat, Utc};

pub fn utc_date() -> String {
    Utc::now().format("%Y-%m-%d").to_string()
}

pub fn utc_timestamp() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamp_is_second_precision_rfc3339() {
        let value = utc_timestamp();
        assert_eq!(value.len(), 20);
        assert_eq!(&value[10..11], "T");
        assert!(value.ends_with('Z'));
    }
}
