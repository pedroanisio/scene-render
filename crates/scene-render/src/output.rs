//! What a command prints: text for a person (the default), or with `--format json` one JSON document, the
//! envelope `scene-render-cli/1` described by `schema/cli-output-1.schema.json`.
//!
//! In JSON mode the text a command writes to [`Out::w`] is discarded; the command hands its machine result to
//! [`Out::set_result`], its findings to [`Out::note`] and its failure to [`Out::fail`] or [`Out::record`], and
//! [`Out::finish`] assembles the envelope. Standard error keeps the one-line messages a person watching the run
//! reads, so standard output holds the document alone.

use std::io::Write;
use std::path::Path;

use anstream::{AutoStream, ColorChoice};
use anstyle::{AnsiColor, Style};
use serde_json::{json, Map, Value};
use sr_model::diag::LineIndex;
use sr_model::{Diagnostic, Report, Severity};

/// The envelope's `format`: its major version is the file name of the schema.
pub const FORMAT: &str = "scene-render-cli/1";

/// The envelope's `schema_version` (semantic versioning): a new optional key is a minor change, a key removed or
/// whose meaning changes is a new `FORMAT`.
pub const SCHEMA_VERSION: &str = "1.2.0";

/// Why a command did not succeed; the exit status follows from it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ErrorKind {
    /// The command line or its arguments cannot be used.
    Usage,
    /// A file could not be read or written.
    Io,
    /// The document is not valid (or has warnings, with `--deny-warnings`).
    Invalid,
    /// The document is valid but the command did not do all it was asked: a frame or output failed, `--strict`
    /// found something not rendered as authored, a target is stale.
    Failed,
    /// What the command needs is missing: a GPU adapter, a tool.
    Unavailable,
    /// A defect in this program (a panic).
    Internal,
}

impl ErrorKind {
    /// The documented exit status: 1 for a document that is invalid or a run that failed, 2 for a run that could
    /// not complete.
    pub fn exit(self) -> u8 {
        match self {
            ErrorKind::Invalid | ErrorKind::Failed => 1,
            ErrorKind::Usage | ErrorKind::Io | ErrorKind::Unavailable | ErrorKind::Internal => 2,
        }
    }
}

/// The envelope's `error`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ErrorInfo {
    /// What kind of failure.
    pub kind: ErrorKind,
    /// What failed, for a person.
    pub message: String,
    /// The diagnostic codes behind it (`scene-render explain CODE`), first seen first.
    pub codes: Vec<String>,
}

/// Standard output with colour, or nowhere (JSON mode, where standard output holds the envelope alone).
pub enum Sink {
    /// The terminal or pipe.
    Term(AutoStream<std::io::Stdout>),
    /// Text is dropped.
    Discard,
}

impl Write for Sink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            Sink::Term(w) => w.write(buf),
            Sink::Discard => Ok(buf.len()),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Sink::Term(w) => w.flush(),
            Sink::Discard => Ok(()),
        }
    }
}

/// A command's output: text on [`Out::w`], and in JSON mode the parts of the envelope.
pub struct Out {
    /// Text output; discarded in JSON mode.
    pub w: Sink,
    json: bool,
    echo: bool,
    result: Option<Value>,
    diagnostics: Vec<Value>,
    error: Option<ErrorInfo>,
}

pub(crate) fn style(c: AnsiColor, bold: bool) -> Style {
    let s = Style::new().fg_color(Some(c.into()));
    if bold {
        s.bold()
    } else {
        s
    }
}

impl Out {
    /// Output for the command line: text on standard output, or in JSON mode the envelope there and the messages
    /// on standard error.
    pub fn new(color: ColorChoice, json: bool) -> Self {
        let w = if json { Sink::Discard } else { Sink::Term(AutoStream::new(std::io::stdout(), color)) };
        Out { w, json, echo: true, result: None, diagnostics: Vec::new(), error: None }
    }

    /// JSON mode with nothing printed at all: the caller takes the envelope from [`Out::finish`].
    pub fn capture() -> Self {
        Out { w: Sink::Discard, json: true, echo: false, result: None, diagnostics: Vec::new(), error: None }
    }

    /// Whether the command builds the envelope rather than text.
    pub fn json(&self) -> bool {
        self.json
    }

    /// Whether messages go to standard error.
    pub fn echo(&self) -> bool {
        self.echo
    }

    /// A finding about `file`: rustc-style text, or in JSON mode an entry of the envelope's `diagnostics`.
    pub fn diagnostic(&mut self, file: &Path, lines: &LineIndex, d: &Diagnostic) -> std::io::Result<()> {
        if self.json {
            self.note(file, d);
            return Ok(());
        }
        let (label, color) = match d.severity {
            Severity::Error => ("error", AnsiColor::Red),
            Severity::Warning => ("warning", AnsiColor::Yellow),
            Severity::Info => ("info", AnsiColor::Cyan),
        };
        let head = style(color, true);
        let bold = Style::new().bold();
        let blue = style(AnsiColor::Blue, true);
        let context = d.label.as_ref().map(|s| format!(" ({s})")).unwrap_or_default();
        writeln!(self.w, "{head}{label}[{}]{head:#}{bold}{context}: {}{bold:#}", d.code, d.message)?;
        let gutter = d.loc.line.max(1).to_string().len();
        let pad = " ".repeat(gutter);
        writeln!(self.w, "{pad}{blue}-->{blue:#} {}:{}:{}", file.display(), d.loc.line, d.loc.column)?;
        let text = lines.line_text(d.loc.line);
        if d.loc.line > 0 && !text.is_empty() {
            let shown: String = text.chars().map(|c| if c == '\t' { ' ' } else { c }).collect();
            writeln!(self.w, "{pad} {blue}|{blue:#}")?;
            writeln!(self.w, "{blue}{}{blue:#} {blue}|{blue:#} {shown}", d.loc.line)?;
            let col = d.loc.column.max(1) as usize - 1;
            let span =
                shown.chars().skip(col).take_while(|c| !c.is_whitespace() && *c != '>' && *c != '/').count().max(1);
            writeln!(self.w, "{pad} {blue}|{blue:#} {}{head}{}{head:#}", " ".repeat(col), "^".repeat(span))?;
        }
        if let Some(h) = &d.help {
            writeln!(self.w, "{pad} {blue}={blue:#} {bold}help{bold:#}: {h}")?;
        }
        writeln!(self.w, "{pad} {blue}={blue:#} at {}", d.path)?;
        writeln!(self.w)
    }

    /// Records a finding about `file` in the envelope's `diagnostics`, and says it on standard error in one line.
    pub fn note(&mut self, file: &Path, d: &Diagnostic) {
        if self.echo {
            eprintln!("{}[{}]: {}", d.severity, d.code, d.message);
        }
        let mut v = serde_json::to_value(d).unwrap_or_else(|_| json!({}));
        if let Value::Object(m) = &mut v {
            m.insert("file".into(), Value::String(file.display().to_string()));
        }
        self.diagnostics.push(v);
    }

    /// The `valid`/`invalid` line after a file's diagnostics; nothing in JSON mode.
    pub fn summary(&mut self, file: &Path, report: &Report, deny_warnings: bool) -> std::io::Result<()> {
        if self.json {
            return Ok(());
        }
        let (e, w, i) = (report.error_count(), report.warning_count(), report.info_count());
        let ok = e == 0 && !(deny_warnings && w > 0);
        let (mark, st) =
            if ok { ("valid", style(AnsiColor::Green, true)) } else { ("invalid", style(AnsiColor::Red, true)) };
        let info = if i > 0 { format!(", {i} info") } else { String::new() };
        writeln!(self.w, "{st}{mark}{st:#} {}: {e} error(s), {w} warning(s){info}", file.display())
    }

    /// The command's machine result, the envelope's `result`.
    pub fn set_result(&mut self, v: Value) {
        self.result = Some(v);
    }

    /// Records why the command does not succeed, printing nothing. Several failures keep every message; the
    /// kind of the one with the higher exit status stands (the first, on a tie).
    pub fn record(&mut self, kind: ErrorKind, message: impl Into<String>, codes: Vec<String>) {
        let message = message.into();
        match &mut self.error {
            None => self.error = Some(ErrorInfo { kind, message, codes: unique(codes) }),
            Some(e) => {
                if kind.exit() > e.kind.exit() {
                    e.kind = kind;
                }
                e.message = format!("{}; {message}", e.message);
                let mut all = std::mem::take(&mut e.codes);
                all.extend(codes);
                e.codes = unique(all);
            }
        }
    }

    /// A failure: says `error: MESSAGE` on standard error, records it, and gives the exit status of its kind.
    pub fn fail(&mut self, kind: ErrorKind, message: impl Into<String>) -> u8 {
        let message = message.into();
        if self.echo {
            eprintln!("error: {message}");
        }
        self.record(kind, message, Vec::new());
        kind.exit()
    }

    /// A document that does not load: its diagnostics (text, or the envelope's `diagnostics`) and the failure.
    pub fn report_invalid(&mut self, file: &Path, lines: &LineIndex, report: &Report) -> std::io::Result<u8> {
        for d in &report.diagnostics {
            self.diagnostic(file, lines, d)?;
        }
        self.summary(file, report, false)?;
        let codes = report.diagnostics.iter().filter(|d| d.severity == Severity::Error).map(|d| d.code.clone());
        self.record(
            ErrorKind::Invalid,
            format!("{}: {} error(s)", file.display(), report.error_count()),
            codes.collect(),
        );
        Ok(ErrorKind::Invalid.exit())
    }

    /// The exit status of a run that returned `exit`: the higher of it and that of a recorded failure.
    pub fn exit_status(&self, exit: u8) -> u8 {
        self.error.as_ref().map_or(exit, |e| exit.max(e.kind.exit()))
    }

    /// The envelope of the run of `command` (null when no command was understood) that returned `exit`, with its
    /// exit status; the result, findings and failure are taken, so the next run starts afresh.
    pub fn finish(&mut self, command: Option<&str>, exit: u8) -> (Value, u8) {
        let exit = self.exit_status(exit);
        let recorded = self.error.take();
        let error = (exit != 0).then(|| {
            recorded.unwrap_or_else(|| {
                let kind = if exit == 1 { ErrorKind::Failed } else { ErrorKind::Internal };
                let name = command.unwrap_or("scene-render");
                ErrorInfo { kind, message: format!("{name} exited with status {exit}"), codes: Vec::new() }
            })
        });
        let mut doc = Map::new();
        doc.insert("format".into(), FORMAT.into());
        doc.insert("schema_version".into(), SCHEMA_VERSION.into());
        doc.insert("command".into(), command.map_or(Value::Null, |c| c.into()));
        doc.insert("ok".into(), (exit == 0).into());
        doc.insert("exit_code".into(), exit.into());
        doc.insert("result".into(), self.result.take().unwrap_or(Value::Null));
        doc.insert("diagnostics".into(), Value::Array(std::mem::take(&mut self.diagnostics)));
        doc.insert("error".into(), error.map_or(Value::Null, |e| serde_json::to_value(e).unwrap_or(Value::Null)));
        (Value::Object(doc), exit)
    }

    /// Prints the envelope of the run as one line on standard output and gives its exit status. A reader that
    /// closed the pipe is not an error.
    pub fn emit(&mut self, command: Option<&str>, exit: u8) -> u8 {
        let (doc, exit) = self.finish(command, exit);
        let mut stdout = std::io::stdout().lock();
        let _ = writeln!(stdout, "{doc}").and_then(|()| stdout.flush());
        exit
    }
}

fn unique(codes: Vec<String>) -> Vec<String> {
    let mut seen = Vec::new();
    for c in codes {
        if !seen.contains(&c) {
            seen.push(c);
        }
    }
    seen
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_map_to_the_documented_exit_status() {
        assert_eq!(ErrorKind::Invalid.exit(), 1);
        assert_eq!(ErrorKind::Failed.exit(), 1);
        for k in [ErrorKind::Usage, ErrorKind::Io, ErrorKind::Unavailable, ErrorKind::Internal] {
            assert_eq!(k.exit(), 2, "{k:?}");
        }
    }

    #[test]
    fn a_success_has_no_error_and_a_failure_always_has_one() {
        let mut out = Out::capture();
        out.set_result(json!({"x": 1}));
        let (doc, exit) = out.finish(Some("inspect"), 0);
        assert_eq!(exit, 0);
        assert_eq!(doc["ok"], true);
        assert_eq!(doc["error"], Value::Null);
        assert_eq!(doc["result"]["x"], 1);
        assert_eq!(doc["format"], FORMAT);
        assert_eq!(doc["schema_version"], SCHEMA_VERSION);
        // a nonzero status with nothing recorded still names a failure
        let (doc, exit) = out.finish(Some("gpus"), 2);
        assert_eq!((exit, doc["ok"].clone()), (2, Value::Bool(false)));
        assert_eq!(doc["error"]["kind"], "internal");
        assert_eq!(doc["result"], Value::Null, "the previous run's result was taken");
        let (doc, _) = out.finish(None, 1);
        assert_eq!(doc["error"]["kind"], "failed");
        assert_eq!(doc["command"], Value::Null);
    }

    #[test]
    fn a_recorded_failure_raises_the_exit_status_and_the_worse_kind_stands() {
        let mut out = Out::capture();
        out.record(ErrorKind::Invalid, "a.xml: 1 error(s)", vec!["S01".into(), "S01".into()]);
        assert_eq!(out.fail(ErrorKind::Io, "cannot read b.xml"), 2);
        out.record(ErrorKind::Failed, "late", vec!["C21".into()]);
        let (doc, exit) = out.finish(Some("validate"), 0);
        assert_eq!(exit, 2);
        assert_eq!(doc["error"]["kind"], "io");
        assert_eq!(doc["error"]["message"], "a.xml: 1 error(s); cannot read b.xml; late");
        assert_eq!(doc["error"]["codes"], json!(["S01", "C21"]));
    }

    #[test]
    fn findings_carry_their_file_and_text_is_discarded() {
        let mut out = Out::capture();
        let d = Diagnostic::error("S01", "bad", sr_model::diag::Loc { line: 2, column: 3, offset: 9 }, "/scene");
        out.diagnostic(Path::new("a.xml"), &LineIndex::new("<scene>\n<x/>\n"), &d).unwrap();
        writeln!(out.w, "dropped").unwrap();
        let (doc, _) = out.finish(Some("eval"), 0);
        assert_eq!(doc["diagnostics"][0]["file"], "a.xml");
        assert_eq!(doc["diagnostics"][0]["code"], "S01");
        assert_eq!(doc["diagnostics"][0]["loc"]["line"], 2);
    }
}
