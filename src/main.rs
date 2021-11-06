#[macro_use]
extern crate concat_with;
extern crate clap;
extern crate terminal_size;

extern crate tokio;

extern crate dnsclient;

#[cfg(feature = "json")]
extern crate serde;

#[cfg(feature = "json")]
#[macro_use]
extern crate serde_derive;

#[cfg(feature = "json")]
extern crate serde_json;

#[cfg(any(unix, feature = "json"))]
extern crate path_absolutize;

use std::error::Error;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::process::{self, Command};
use std::str::FromStr;
use std::time::Duration;

#[cfg(any(unix, feature = "json"))]
use std::path::Path;

#[cfg(unix)]
use tokio::net::UnixStream;

#[cfg(feature = "json")]
use tokio::fs;
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::time::{self, sleep, Instant};

use dnsclient::r#async::DNSClient;
use dnsclient::UpstreamServer;

#[cfg(any(unix, feature = "json"))]
use path_absolutize::Absolutize;

use clap::{App, Arg, ArgMatches, Values};
use terminal_size::terminal_size;

const APP_NAME: &str = "wait-service";
const CARGO_PKG_VERSION: &str = env!("CARGO_PKG_VERSION");
const CARGO_PKG_AUTHORS: &str = env!("CARGO_PKG_AUTHORS");

const DEFAULT_TIMEOUT_SECONDS: &str = "60";
const SLEEP_INTERVAL: Duration = Duration::from_millis(500);

#[cfg_attr(feature = "json", derive(Deserialize))]
#[derive(Debug)]
struct TcpTask {
    host: String,
    port: u16,
}

#[cfg(unix)]
#[cfg_attr(feature = "json", derive(Deserialize))]
#[derive(Debug)]
struct UdsTask {
    uds: String,
}

#[cfg(feature = "json")]
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Task {
    Tcp(TcpTask),
    #[cfg(unix)]
    Uds(UdsTask),
}

#[inline]
fn exec(sources: Vec<&str>) -> Result<(), Box<dyn Error>> {
    let mut iter = sources.into_iter();

    let mut command = Command::new(iter.next().unwrap());

    command.args(iter);

    let exit_status = command.spawn()?.wait()?;

    process::exit(exit_status.code().unwrap_or(-1));
}

#[inline]
fn handle_command(values: Option<Values>) -> Result<Vec<&str>, &'static str> {
    match values {
        Some(values) => Ok(values.collect()),
        None => Err("A command is needed."),
    }
}

#[inline]
async fn host_port_to_socket_addrs(
    host: &str,
    port: u16,
) -> Result<Vec<SocketAddr>, Box<dyn Error>> {
    match IpAddr::from_str(host) {
        Ok(ip) => Ok(vec![SocketAddr::new(ip, port)]),
        Err(_) => {
            let dns_servers = vec![
                UpstreamServer::new(SocketAddr::new(IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)), 53)),
                UpstreamServer::new(SocketAddr::new(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)), 53)),
                UpstreamServer::new(SocketAddr::new(IpAddr::V4(Ipv4Addr::new(4, 4, 4, 4)), 53)),
            ];

            #[cfg(unix)]
            let client = match DNSClient::new_with_system_resolvers() {
                Ok(client) => client,
                Err(_) => DNSClient::new(dns_servers),
            };

            #[cfg(windows)]
            let client = DNSClient::new(dns_servers);

            Ok(client
                .query_addrs(host)
                .await?
                .into_iter()
                .map(|ip| SocketAddr::new(ip, port))
                .collect())
        }
    }
}

async fn wait_tcp(tcp_task: &TcpTask, timeout: Duration) -> Result<(), String> {
    if timeout.is_zero() {
        let addrs = host_port_to_socket_addrs(tcp_task.host.as_str(), tcp_task.port)
            .await
            .map_err(|err| format!("Cannot resolve the host: {:?} {}", tcp_task.host, err))?;

        'outer: loop {
            for addr in addrs.iter().cloned() {
                if TcpStream::connect(addr).await.is_ok() {
                    break 'outer;
                }
            }

            sleep(SLEEP_INTERVAL).await;
        }
    } else {
        let addrs = time::timeout(
            timeout,
            host_port_to_socket_addrs(tcp_task.host.as_str(), tcp_task.port),
        )
        .await
        .map_err(|_| format!("Cannot resolve to host: {:?} timeout", tcp_task.host))?
        .map_err(|err| format!("Cannot resolve the host: {:?} {}", tcp_task.host, err))?;

        let start = Instant::now();

        'outer_timeout: loop {
            for addr in addrs.iter().cloned() {
                match time::timeout(timeout, TcpStream::connect(addr))
                    .await
                    .map_err(|_| format!("Cannot connect to the address: {} timeout", addr))?
                {
                    Ok(_) => break 'outer_timeout,
                    Err(err) => {
                        if Instant::now() - start > timeout {
                            return Err(format!("Cannot connect to the address: {} {}", addr, err));
                        }
                    }
                }
            }

            sleep(SLEEP_INTERVAL).await;
        }
    }

    Ok(())
}

#[cfg(unix)]
async fn wait_uds(uds_task: &UdsTask, timeout: Duration) -> Result<(), String> {
    if timeout.is_zero() {
        while UnixStream::connect(uds_task.uds.as_str()).await.is_err() {
            sleep(SLEEP_INTERVAL).await;
        }
    } else {
        let start = Instant::now();

        while let Err(err) = time::timeout(timeout, UnixStream::connect(uds_task.uds.as_str()))
            .await
            .map_err(|_| {
                format!(
                    "Cannot connect to the socket: {:?} timeout",
                    Path::new(uds_task.uds.as_str()).absolutize().unwrap().to_string_lossy()
                )
            })?
        {
            if Instant::now() - start > timeout {
                return Err(format!(
                    "Cannot connect to the socket: {:?} {}",
                    Path::new(uds_task.uds.as_str()).absolutize().unwrap().to_string_lossy(),
                    err
                ));
            } else {
                sleep(SLEEP_INTERVAL).await;
            }
        }
    }

    Ok(())
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn Error>> {
    let matches = get_matches();

    let timeout = Duration::from_secs(matches.value_of("TIMEOUT").unwrap().parse::<u32>()? as u64);

    let command = handle_command(matches.values_of("COMMAND"))?;

    let mut tcp_tasks = Vec::new();

    #[cfg(unix)]
    let mut uds_tasks = Vec::new();

    if let Some(tcp) = matches.values_of("TCP") {
        tcp_tasks.reserve(tcp.len());

        for e in tcp {
            let (host, port) = match e.rfind(':') {
                Some(i) => {
                    (
                        &e[..i],
                        e[(i + 1)..]
                            .parse::<u16>()
                            .map_err(|_| format!("{} is not a correct port!", &e[(i + 1)..]))?,
                    )
                }
                None => return Err(format!("{} needs to have a port!", e).into()),
            };

            tcp_tasks.push(TcpTask {
                host: String::from(host),
                port,
            });
        }
    }

    #[cfg(unix)]
    if let Some(uds) = matches.values_of("UDS") {
        uds_tasks.reserve(uds.len());

        for e in uds {
            uds_tasks.push(UdsTask {
                uds: String::from(e),
            });
        }
    }

    #[cfg(feature = "json")]
    if let Some(json) = matches.values_of("JSON") {
        for json_path in json.map(Path::new) {
            let tasks: Vec<Task> = serde_json::from_str(
                fs::read_to_string(json_path)
                    .await
                    .map_err(|err| {
                        format!(
                            "{:?} cannot be successfully read. {}",
                            json_path.absolutize().unwrap().to_string_lossy(),
                            err
                        )
                    })?
                    .as_str(),
            )
            .map_err(|err| {
                format!(
                    "{:?} is not a correct service list file: {}",
                    json_path.absolutize().unwrap().to_string_lossy(),
                    err
                )
            })?;

            for task in tasks {
                match task {
                    Task::Tcp(task) => tcp_tasks.push(task),
                    #[cfg(unix)]
                    Task::Uds(task) => uds_tasks.push(task),
                }
            }
        }
    }

    let task_count = tcp_tasks.len() + uds_tasks.len();

    if task_count == 0 {
        eprintln!("Warning: \"No service to wait.\"");

        return exec(command);
    }

    let (sender, mut receiver) = mpsc::channel(task_count);

    for tcp_task in tcp_tasks {
        let sender = sender.clone();

        tokio::spawn(async move {
            match wait_tcp(&tcp_task, timeout).await {
                Ok(_) => {
                    sender.send(true).await.unwrap();
                }
                Err(err) => {
                    eprintln!("{}", err);

                    sender.send(false).await.unwrap();
                }
            }
        });
    }

    for uds_task in uds_tasks {
        let sender = sender.clone();

        tokio::spawn(async move {
            match wait_uds(&uds_task, timeout).await {
                Ok(_) => {
                    sender.send(true).await.unwrap();
                }
                Err(err) => {
                    eprintln!("{}", err);

                    sender.send(false).await.unwrap();
                }
            }
        });
    }

    for _ in 0..task_count {
        let result = receiver.recv().await.unwrap();

        if !result {
            process::exit(-1);
        }
    }

    exec(command)
}

fn get_matches<'a>() -> ArgMatches<'a> {
    let app = App::new(APP_NAME)
        .set_term_width(terminal_size().map(|(width, _)| width.0 as usize).unwrap_or(0))
        .version(CARGO_PKG_VERSION)
        .author(CARGO_PKG_AUTHORS)
        .about(concat!("Wait Service is a pure rust program to test and wait on the availability of multiple services\n\nEXAMPLES:\n", concat_line!(prefix "wait-service ",
            "--tcp localhost:27017 --tcp localhost:27018   -t 5 -- npm start  # Wait for localhost:27017 and localhost:27018 (max 5 seconds) and then run `npm start`",
            "--tcp localhost:27017 --uds /var/run/app.sock -t 0 -- npm start  # Wait for localhost:27017 and /var/run/app.sock (forever) and then run `npm start`",
            "--uds /var/run/app.sock --json /path/to/json       -- npm start  # Wait for /var/run/app.sock and other services defined in the json file (max 60 seconds) and then run `npm start`",
        )));

    let arg_timeout = Arg::with_name("TIMEOUT")
        .display_order(0)
        .required(true)
        .long("timeout")
        .short("t")
        .takes_value(true)
        .default_value(DEFAULT_TIMEOUT_SECONDS)
        .help("Sets the timeout in seconds, zero for no timeout");

    let arg_command = Arg::with_name("COMMAND")
        .required(true)
        .help("Command to execute after service is available")
        .multiple(true);

    let arg_tcp = Arg::with_name("TCP")
        .display_order(1)
        .long("tcp")
        .takes_value(true)
        .help("Test and wait on the availability of TCP services")
        .multiple(true);

    let arg_tcp = if cfg!(unix) {
        if cfg!(feature = "json") {
            arg_tcp.required_unless_one(&["UDS", "JSON"])
        } else {
            arg_tcp.required_unless_one(&["UDS"])
        }
    } else if cfg!(feature = "json") {
        arg_tcp.required_unless_one(&["JSON"])
    } else {
        arg_tcp
    };

    #[cfg(unix)]
    let arg_uds = Arg::with_name("UDS")
        .display_order(2)
        .long("uds")
        .takes_value(true)
        .visible_alias("unix")
        .required_unless_one(if cfg!(feature = "json") {
            ["TCP", "JSON"].as_ref()
        } else {
            ["TCP"].as_ref()
        })
        .help("Test and wait on the availability of UDS services")
        .multiple(true);

    #[cfg(feature = "json")]
    let arg_json = Arg::with_name("JSON")
        .display_order(3)
        .long("json")
        .takes_value(true)
        .required_unless_one(if cfg!(unix) {
            ["TCP", "UDS"].as_ref()
        } else {
            ["TCP"].as_ref()
        })
        .help("Test and wait on the availability of TCP or UDS services")
        .multiple(true);

    let app = app.arg(arg_timeout).arg(arg_command).arg(arg_tcp);

    #[cfg(unix)]
    let app = app.arg(arg_uds);

    #[cfg(feature = "json")]
    let app = app.arg(arg_json);

    app.after_help("Enjoy it! https://magiclen.org").get_matches()
}
