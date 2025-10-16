mod cli;

#[cfg(unix)]
use std::path::PathBuf;
use std::{
    io,
    io::Write,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    process,
    process::Command,
    str::FromStr,
    time::Duration,
};

use anyhow::{anyhow, Context};
use cli::*;
use dnsclient::{r#async::DNSClient, UpstreamServer};
use once_cell::sync::Lazy;
#[cfg(any(unix, feature = "json"))]
use path_absolutize::Absolutize;
#[cfg(feature = "json")]
use serde::Deserialize;
#[cfg(feature = "json")]
use tokio::fs;
#[cfg(unix)]
use tokio::net::UnixStream;
use tokio::{net::TcpStream, sync::mpsc, time, time::sleep};

const SLEEP_INTERVAL: Duration = Duration::from_millis(500);

static DNS_CLIENT: Lazy<DNSClient> = Lazy::new(|| {
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

    client
});

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
    uds: PathBuf,
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
fn exec(sources: Vec<String>) -> anyhow::Result<()> {
    let mut iter = sources.into_iter();

    let mut command = Command::new(iter.next().unwrap());

    command.args(iter);

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;

        Err(command.exec()).with_context(|| anyhow!("{command:?}"))?
    }

    #[cfg(windows)]
    {
        let exit_status = command
            .spawn()
            .with_context(|| anyhow!("{command:?}"))?
            .wait()
            .with_context(|| anyhow!("{command:?}"))?;

        process::exit(exit_status.code().unwrap_or(-1));
    }
}

#[inline]
async fn host_port_to_socket_addrs(host: &str, port: u16) -> anyhow::Result<Vec<SocketAddr>> {
    match IpAddr::from_str(host) {
        Ok(ip) => Ok(vec![SocketAddr::new(ip, port)]),
        Err(_) => Ok(DNS_CLIENT
            .query_addrs(host)
            .await
            .with_context(|| anyhow!("{host:?}"))?
            .into_iter()
            .map(|ip| SocketAddr::new(ip, port))
            .collect()),
    }
}

async fn wait_tcp_handler(tcp_task: &TcpTask) -> anyhow::Result<()> {
    'outer: loop {
        let addrs = host_port_to_socket_addrs(tcp_task.host.as_str(), tcp_task.port).await?;

        for addr in addrs.iter().cloned() {
            if TcpStream::connect(addr).await.is_ok() {
                break 'outer;
            }
        }

        sleep(SLEEP_INTERVAL).await;
    }

    Ok(())
}

#[inline]
async fn wait_tcp(tcp_task: &TcpTask, timeout: Duration) -> anyhow::Result<()> {
    if timeout.is_zero() {
        wait_tcp_handler(tcp_task).await
    } else {
        time::timeout(timeout, wait_tcp_handler(tcp_task)).await.with_context(|| {
            anyhow!("Cannot connect to server: {}:{} timeout.", tcp_task.host, tcp_task.port)
        })?
    }
}

#[cfg(unix)]
async fn wait_uds_handler(uds_task: &UdsTask) {
    while UnixStream::connect(uds_task.uds.as_path()).await.is_err() {
        sleep(SLEEP_INTERVAL).await;
    }
}

#[cfg(unix)]
async fn wait_uds(uds_task: &UdsTask, timeout: Duration) -> anyhow::Result<()> {
    if timeout.is_zero() {
        wait_uds_handler(uds_task).await
    } else {
        time::timeout(timeout, wait_uds_handler(uds_task)).await.with_context(|| {
            anyhow!(
                "Cannot connect to the socket: {:?} timeout.",
                uds_task.uds.absolutize().unwrap()
            )
        })?;
    }

    Ok(())
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let args = get_args();

    let timeout = Duration::from_secs(args.timeout);

    let mut tcp_tasks = Vec::new();

    #[cfg(unix)]
    let mut uds_tasks = Vec::new();

    {
        tcp_tasks.reserve(args.tcp.len());

        for e in args.tcp {
            let (host, port) = match e.rfind(':') {
                Some(i) => {
                    (&e[..i], e[(i + 1)..].parse::<u16>().with_context(|| anyhow!("{e:?}"))?)
                },
                None => return Err(anyhow!("{e:?} needs to have a port!")),
            };

            tcp_tasks.push(TcpTask {
                host: String::from(host),
                port,
            });
        }
    }

    #[cfg(unix)]
    {
        uds_tasks.reserve(args.uds.len());

        for e in args.uds {
            uds_tasks.push(UdsTask {
                uds: e
            });
        }
    }

    #[cfg(feature = "json")]
    {
        for json_path in args.json {
            let tasks: Vec<Task> = serde_json::from_str(
                fs::read_to_string(json_path.as_path())
                    .await
                    .with_context(|| {
                        anyhow!(
                            "{:?} cannot be successfully read.",
                            json_path.absolutize().unwrap()
                        )
                    })?
                    .as_str(),
            )
            .with_context(|| {
                anyhow!("{:?} is not a correct service list file", json_path.absolutize().unwrap(),)
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

    #[cfg(unix)]
    let task_count = tcp_tasks.len() + uds_tasks.len();

    #[cfg(windows)]
    let task_count = tcp_tasks.len();

    if task_count == 0 {
        eprintln!("Warning: \"No service to wait.\"");

        return exec(args.command);
    }

    let (sender, mut receiver) = mpsc::channel(task_count);

    for tcp_task in tcp_tasks {
        let sender = sender.clone();

        tokio::spawn(async move {
            match wait_tcp(&tcp_task, timeout).await {
                Ok(_) => {
                    sender.send(true).await.unwrap();
                },
                Err(error) => {
                    eprintln!("{error:?}");
                    io::stderr().flush().unwrap();

                    sender.send(false).await.unwrap();
                },
            }
        });
    }

    #[cfg(unix)]
    for uds_task in uds_tasks {
        let sender = sender.clone();

        tokio::spawn(async move {
            match wait_uds(&uds_task, timeout).await {
                Ok(_) => {
                    sender.send(true).await.unwrap();
                },
                Err(error) => {
                    eprintln!("{error:?}");
                    io::stderr().flush().unwrap();

                    sender.send(false).await.unwrap();
                },
            }
        });
    }

    for _ in 0..task_count {
        let result = receiver.recv().await.unwrap();

        if !result {
            process::exit(-1);
        }
    }

    exec(args.command)
}
