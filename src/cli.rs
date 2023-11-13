#[cfg(any(unix, feature = "json"))]
use std::path::PathBuf;

use clap::{CommandFactory, FromArgMatches, Parser};
use concat_with::concat_line;
use terminal_size::terminal_size;

const APP_NAME: &str = "wait-service";
const CARGO_PKG_VERSION: &str = env!("CARGO_PKG_VERSION");
const CARGO_PKG_AUTHORS: &str = env!("CARGO_PKG_AUTHORS");

const AFTER_HELP: &str = "Enjoy it! https://magiclen.org";

const APP_ABOUT: &str = concat!(
    "Wait Service is a pure rust program to test and wait on the availability of multiple \
     services\n\nEXAMPLES:\n",
    concat_line!(prefix "wait-service ",
        "--tcp localhost:27017 --tcp localhost:27018   -t 5 -- npm start   # Wait for localhost:27017 and localhost:27018 (max 5 seconds) and then run `npm start`",
        "--tcp localhost:27017 --uds /var/run/app.sock -t 0 -- npm start   # Wait for localhost:27017 and /var/run/app.sock (forever) and then run `npm start`",
        "--uds /var/run/app.sock --json /path/to/json       -- npm start   # Wait for /var/run/app.sock and other services defined in the json file (max 60 seconds) and then run `npm start`",
    )
);

#[derive(Debug, Parser)]
#[command(name = APP_NAME)]
#[command(term_width = terminal_size().map(|(width, _)| width.0 as usize).unwrap_or(0))]
#[command(version = CARGO_PKG_VERSION)]
#[command(author = CARGO_PKG_AUTHORS)]
#[command(after_help = AFTER_HELP)]
pub struct CLIArgs {
    #[arg(short, long)]
    #[arg(default_value = "60")]
    #[arg(help = "Set the timeout in seconds, zero for no timeout")]
    pub timeout: u64,

    #[arg(required = true)]
    #[arg(last = true)]
    #[arg(value_hint = clap::ValueHint::CommandWithArguments)]
    #[arg(help = "Command to execute after service is available")]
    pub command: Vec<String>,

    #[arg(long)]
    #[arg(num_args = 1..)]
    #[cfg_attr(unix, cfg_attr(feature = "json", arg(required_unless_present_any = ["uds", "json"], required_unless_present = "uds")), cfg_attr(feature = "json", arg(required_unless_present = "json")))]
    #[arg(help = "Test and wait on the availability of TCP services")]
    pub tcp: Vec<String>,

    #[cfg(unix)]
    #[arg(long, visible_alias = "unix")]
    #[arg(num_args = 1..)]
    #[cfg_attr(feature = "json", arg(required_unless_present_any = ["tcp", "json"]), arg(required_unless_present = "tcp"))]
    #[arg(value_hint = clap::ValueHint::FilePath)]
    #[arg(help = "Test and wait on the availability of UDS services")]
    pub uds: Vec<PathBuf>,

    #[cfg(feature = "json")]
    #[arg(long)]
    #[arg(num_args = 1..)]
    #[cfg_attr(unix, arg(required_unless_present_any = ["tcp", "uds"]), arg(required_unless_present = "tcp"))]
    #[arg(value_hint = clap::ValueHint::FilePath)]
    #[arg(help = "Test and wait on the availability of TCP or UDS services")]
    pub json: Vec<PathBuf>,
}

pub fn get_args() -> CLIArgs {
    let args = CLIArgs::command();

    let about = format!("{APP_NAME} {CARGO_PKG_VERSION}\n{CARGO_PKG_AUTHORS}\n{APP_ABOUT}");

    let args = args.about(about);

    let matches = args.get_matches();

    match CLIArgs::from_arg_matches(&matches) {
        Ok(args) => args,
        Err(err) => {
            err.exit();
        },
    }
}
