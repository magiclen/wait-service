#[macro_use]
extern crate concat_with;
extern crate clap;
extern crate terminal_size;

extern crate tokio;

use std::error::Error;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

#[cfg(unix)]
use tokio::net::UnixStream;

use tokio::net::TcpStream;

use tokio::time::{self, sleep, Instant};

use clap::{App, Arg, ArgMatches, SubCommand, Values};
use terminal_size::terminal_size;

const APP_NAME: &str = "wait-service";
const CARGO_PKG_VERSION: &str = env!("CARGO_PKG_VERSION");
const CARGO_PKG_AUTHORS: &str = env!("CARGO_PKG_AUTHORS");

const DEFAULT_TIMEOUT_SECONDS: &str = "60";
const SLEEP_INTERVAL: Duration = Duration::from_secs(1);

fn exec(sources: Vec<&str>) -> Result<(), Box<dyn Error>> {
    let mut iter = sources.into_iter();

    let mut command = Command::new(iter.next().unwrap());

    command.args(iter);

    command.spawn()?;

    Ok(())
}

async fn tcp(
    host: &str,
    port: u16,
    timeout: Duration,
    command: Vec<&str>,
) -> Result<(), Box<dyn Error>> {
    let host_with_port = format!("{}:{}", host, port);

    let start = Instant::now();

    if timeout.is_zero() {
        while TcpStream::connect(host_with_port.as_str()).await.is_err() {
            sleep(SLEEP_INTERVAL).await;
        }
    } else {
        while let Err(err) =
            time::timeout(timeout, TcpStream::connect(host_with_port.as_str())).await?
        {
            if Instant::now() - start > timeout {
                return Err(err.into());
            } else {
                sleep(SLEEP_INTERVAL).await;
            }
        }
    }

    exec(command)?;

    Ok(())
}

#[cfg(unix)]
async fn uds(path: &Path, timeout: Duration, command: Vec<&str>) -> Result<(), Box<dyn Error>> {
    let start = Instant::now();

    if timeout.is_zero() {
        while UnixStream::connect(path).await.is_err() {
            sleep(SLEEP_INTERVAL).await;
        }
    } else {
        while let Err(err) = time::timeout(timeout, UnixStream::connect(path)).await? {
            if Instant::now() - start > timeout {
                return Err(err.into());
            } else {
                sleep(SLEEP_INTERVAL).await;
            }
        }
    }

    exec(command)?;

    Ok(())
}

fn handle_command(values: Option<Values>) -> Result<Vec<&str>, &'static str> {
    match values {
        Some(values) => Ok(values.collect()),
        None => Err("A command is needed."),
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    let matches = get_matches();

    if let Some(sub_matches) = matches.subcommand_matches("tcp") {
        let host = sub_matches.value_of("HOST").unwrap();
        let port = sub_matches.value_of("PORT").unwrap().parse::<u16>()?;
        let timeout =
            Duration::from_secs(sub_matches.value_of("TIMEOUT").unwrap().parse::<u32>()? as u64);

        let command = handle_command(sub_matches.values_of("COMMAND"))?;

        tcp(host, port, timeout, command).await
    } else if let Some(sub_matches) = matches.subcommand_matches("uds") {
        let path = sub_matches.value_of("PATH").unwrap();
        let timeout =
            Duration::from_secs(sub_matches.value_of("TIMEOUT").unwrap().parse::<u32>()? as u64);

        let command = handle_command(sub_matches.values_of("COMMAND"))?;

        uds(Path::new(path), timeout, command).await
    } else {
        Err("Please input a subcommand. Use `help` to see how to use this program.".into())
    }
}

fn get_matches<'a>() -> ArgMatches<'a> {
    let app = App::new(APP_NAME)
        .set_term_width(terminal_size().map(|(width, _)| width.0 as usize).unwrap_or(0))
        .version(CARGO_PKG_VERSION)
        .author(CARGO_PKG_AUTHORS)
        .about(concat!("Wait Service is a pure rust program to test and wait on the availability of a service\n\nEXAMPLES:\n", concat_line!(prefix "wait-service ",
            "tcp -h localhost -p 27017 -t 5 -- npm start   # Wait for localhost:27017 (max 5 seconds) and then run `npm start`",
            "uds -p /var/run/app.sock -t 0 -- npm start    # Wait for /var/run/app.sock (forever) and then run `npm start`",
        )));

    let arg_timeout = Arg::with_name("TIMEOUT")
        .required(true)
        .long("timeout")
        .short("t")
        .default_value(DEFAULT_TIMEOUT_SECONDS)
        .help("Sets the timeout in seconds, zero for no timeout");

    let arg_command = Arg::with_name("COMMAND")
        .help("Command to execute after service is available")
        .multiple(true);

    let app = app.subcommand(
        SubCommand::with_name("tcp")
            .usage(
                "wait-service tcp --host <HOST> --port <PORT> --timeout <TIMEOUT> -- [COMMAND]...",
            )
            .about("Test and wait on the availability of a TCP service")
            .arg(
                Arg::with_name("HOST")
                    .required(true)
                    .long("host")
                    .short("h")
                    .takes_value(true)
                    .help("Sets the host of the service to be watched"),
            )
            .arg(
                Arg::with_name("PORT")
                    .required(true)
                    .long("port")
                    .short("p")
                    .takes_value(true)
                    .help("Sets the port of the service to be watched"),
            )
            .arg(arg_timeout.clone())
            .arg(arg_command.clone()),
    );

    #[cfg(unix)]
    let app = app.subcommand(
        SubCommand::with_name("uds")
            .visible_alias("unix")
            .usage("wait-service uds --path <PATH> --timeout <TIMEOUT> -- [COMMAND]...")
            .about("Test and wait on the availability of a UDS service")
            .arg(
                Arg::with_name("PATH")
                    .required(true)
                    .long("path")
                    .short("p")
                    .takes_value(true)
                    .help("Sets the path of the socket to be watched"),
            )
            .arg(arg_timeout.clone())
            .arg(arg_command.clone()),
    );

    app.after_help("Enjoy it! https://magiclen.org").get_matches()
}