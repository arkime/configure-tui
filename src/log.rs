//! Structured log lines produced by the apply step and rendered on the Progress
//! screen.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogLine {
    pub level: Level,
    pub text: String,
}

impl LogLine {
    pub fn new(level: Level, text: String) -> Self {
        LogLine { level, text }
    }
}
