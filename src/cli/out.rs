use chrono::{Local, TimeZone};
use serde_json::Value;

/// Output conventions shared by the commands.
pub struct Out {
    pub json: bool,
}

impl Out {
    pub fn new(json: bool) -> Out {
        Out { json }
    }

    /// Prints a JSON object with the schema version added.
    pub fn emit(&self, mut value: Value) {
        if let Value::Object(map) = &mut value {
            map.insert("v".into(), 1.into());
        }
        println!("{value}");
    }
}

/// Collects text lines up to a budget and says how to see the rest.
pub struct Budget {
    pub max: usize,
    pub used: usize,
    pub dropped: usize,
}

impl Budget {
    pub fn new(max: usize) -> Budget {
        Budget {
            max,
            used: 0,
            dropped: 0,
        }
    }

    pub fn line(&mut self, text: &str) {
        if self.used < self.max {
            println!("{text}");
            self.used += 1;
        } else {
            self.dropped += 1;
        }
    }

    pub fn block(&mut self, text: &str) {
        for line in text.lines() {
            self.line(line);
        }
    }

    pub fn finish(&self, hint: &str) {
        if self.dropped > 0 {
            println!("... {} more lines not shown ({hint})", self.dropped);
        }
    }
}

pub fn time_of(ts_ns: i64) -> String {
    Local
        .timestamp_nanos(ts_ns)
        .format("%Y-%m-%d %H:%M:%S")
        .to_string()
}

pub fn clock_of(ts_ns: i64) -> String {
    Local.timestamp_nanos(ts_ns).format("%H:%M:%S").to_string()
}
