use std::fmt;
use std::path::PathBuf;

mod codegen;
mod dist;
mod ext;
mod profile;

const HELP_TEXT: &str = "xtask commands:\n  help\n  codegen [--check]\n  profile [--out PATH]\n  dist [--out-dir DIR]\n  ext [--target <vscode-target>] [--out PATH]";

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Help,
    Codegen {
        check: bool,
    },
    Profile {
        out: Option<PathBuf>,
    },
    Dist {
        out_dir: Option<PathBuf>,
    },
    Ext {
        target: Option<String>,
        out: Option<PathBuf>,
    },
}

#[derive(Debug, PartialEq, Eq)]
enum ParseFlagResult {
    None,
    Value(PathBuf),
    Help,
}

#[derive(Debug, PartialEq, Eq)]
enum ParseExtResult {
    Help,
    Args {
        target: Option<String>,
        out: Option<PathBuf>,
    },
}

#[derive(Debug)]
pub struct XtaskError {
    message: String,
}

impl XtaskError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for XtaskError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for XtaskError {}

pub fn parse_args<I, S>(args: I) -> Result<Command, XtaskError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut iter = args.into_iter();
    let Some(command) = iter.next() else {
        return Ok(Command::Help);
    };

    match command.as_ref() {
        "help" | "--help" | "-h" => Ok(Command::Help),
        "codegen" => {
            let mut check = false;
            for arg in iter {
                match arg.as_ref() {
                    "--check" => check = true,
                    "--help" | "-h" => return Ok(Command::Help),
                    other => {
                        return Err(XtaskError::new(format!(
                            "unexpected argument for codegen: {other}"
                        )));
                    }
                }
            }
            Ok(Command::Codegen { check })
        }
        "profile" => {
            let out = match parse_optional_flag_value(&mut iter, "--out", "profile")? {
                ParseFlagResult::Help => return Ok(Command::Help),
                ParseFlagResult::Value(value) => Some(value),
                ParseFlagResult::None => None,
            };
            Ok(Command::Profile { out })
        }
        "dist" => {
            let out_dir = match parse_optional_flag_value(&mut iter, "--out-dir", "dist")? {
                ParseFlagResult::Help => return Ok(Command::Help),
                ParseFlagResult::Value(value) => Some(value),
                ParseFlagResult::None => None,
            };
            Ok(Command::Dist { out_dir })
        }
        "ext" => {
            let parsed = parse_ext_flags(&mut iter)?;
            match parsed {
                ParseExtResult::Help => Ok(Command::Help),
                ParseExtResult::Args { target, out } => Ok(Command::Ext { target, out }),
            }
        }
        other => Err(XtaskError::new(format!("unknown command: {other}"))),
    }
}

pub fn run<I, S>(args: I) -> Result<(), XtaskError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    match parse_args(args)? {
        Command::Help => {
            println!("{HELP_TEXT}");
            Ok(())
        }
        Command::Codegen { check } => codegen::run(check),
        Command::Profile { out } => profile::run(out),
        Command::Dist { out_dir } => dist::run(out_dir),
        Command::Ext { target, out } => ext::run(target, out),
    }
}

pub fn help_text() -> &'static str {
    HELP_TEXT
}

pub(crate) fn workspace_root() -> Result<PathBuf, XtaskError> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent()
        .map(|path| path.to_path_buf())
        .ok_or_else(|| XtaskError::new("xtask must live under the workspace root"))
}

fn parse_optional_flag_value<I, S>(
    iter: &mut I,
    flag: &str,
    command: &str,
) -> Result<ParseFlagResult, XtaskError>
where
    I: Iterator<Item = S>,
    S: AsRef<str>,
{
    let mut value = None;
    while let Some(arg) = iter.next() {
        match arg.as_ref() {
            "--help" | "-h" => return Ok(ParseFlagResult::Help),
            other if other == flag => {
                if value.is_some() {
                    return Err(XtaskError::new(format!("duplicate {flag} flag")));
                }
                let value_arg = iter
                    .next()
                    .ok_or_else(|| XtaskError::new(format!("missing value for {flag}")))?;
                let value_str = value_arg.as_ref();
                if value_str.trim().is_empty() {
                    return Err(XtaskError::new(format!("missing value for {flag}")));
                }
                value = Some(PathBuf::from(value_str));
            }
            other => {
                return Err(XtaskError::new(format!(
                    "unexpected argument for {command}: {other}"
                )));
            }
        }
    }

    Ok(match value {
        Some(value) => ParseFlagResult::Value(value),
        None => ParseFlagResult::None,
    })
}

fn parse_ext_flags<I, S>(iter: &mut I) -> Result<ParseExtResult, XtaskError>
where
    I: Iterator<Item = S>,
    S: AsRef<str>,
{
    let mut target = None;
    let mut out = None;

    while let Some(arg) = iter.next() {
        match arg.as_ref() {
            "--help" | "-h" => return Ok(ParseExtResult::Help),
            "--target" => {
                if target.is_some() {
                    return Err(XtaskError::new("duplicate --target flag"));
                }
                let value = iter
                    .next()
                    .ok_or_else(|| XtaskError::new("missing value for --target"))?;
                let value_str = value.as_ref();
                if value_str.trim().is_empty() {
                    return Err(XtaskError::new("missing value for --target"));
                }
                if value_str.starts_with('-') {
                    return Err(XtaskError::new(format!(
                        "invalid value for --target: {value_str}"
                    )));
                }
                target = Some(value_str.to_string());
            }
            "--out" => {
                if out.is_some() {
                    return Err(XtaskError::new("duplicate --out flag"));
                }
                let value = iter
                    .next()
                    .ok_or_else(|| XtaskError::new("missing value for --out"))?;
                let value_str = value.as_ref();
                if value_str.trim().is_empty() {
                    return Err(XtaskError::new("missing value for --out"));
                }
                if value_str.starts_with('-') {
                    return Err(XtaskError::new(format!(
                        "invalid value for --out: {value_str}"
                    )));
                }
                out = Some(PathBuf::from(value_str));
            }
            other => {
                return Err(XtaskError::new(format!(
                    "unexpected argument for ext: {other}"
                )));
            }
        }
    }

    Ok(ParseExtResult::Args { target, out })
}
