mod cli;

#[cfg(any(unix, feature = "json"))]
use std::path::PathBuf;
use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    process,
    process::Command,
    str::FromStr,
    sync::LazyLock,
    time::Duration,
};

use anyhow::{Context, anyhow};
use cli::*;
use dnsclient::{UpstreamServer, r#async::DNSClient};
#[cfg(any(unix, feature = "json"))]
use path_absolutize::Absolutize;
#[cfg(feature = "json")]
use serde::Deserialize;
#[cfg(feature = "json")]
use tokio::fs;
#[cfg(unix)]
use tokio::net::UnixStream;
use tokio::{
    net::{TcpStream, lookup_host},
    sync::mpsc,
    time,
    time::sleep,
};

const SLEEP_INTERVAL: Duration = Duration::from_millis(500);
/// A single connection attempt is bounded so that one unreachable address cannot eat up the whole timeout.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

static DNS_CLIENT: LazyLock<DNSClient> = LazyLock::new(|| {
    let dns_servers = vec![
        UpstreamServer::new(SocketAddr::new(IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)), 53)),
        UpstreamServer::new(SocketAddr::new(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)), 53)),
        UpstreamServer::new(SocketAddr::new(IpAddr::V4(Ipv4Addr::new(8, 8, 4, 4)), 53)),
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

/// Parses a `host:port` argument. An IPv6 literal has to be bracketed, e.g. `[::1]:8080`.
fn parse_tcp_arg(s: &str) -> anyhow::Result<TcpTask> {
    if let Ok(addr) = SocketAddr::from_str(s) {
        return Ok(TcpTask {
            host: addr.ip().to_string(), port: addr.port()
        });
    }

    let i = s.rfind(':').ok_or_else(|| anyhow!("{s:?} needs to have a port!"))?;

    let raw_host = &s[..i];
    let host = raw_host.strip_prefix('[').and_then(|e| e.strip_suffix(']')).unwrap_or(raw_host);

    if host.is_empty() {
        return Err(anyhow!("{s:?} needs to have a host!"));
    }

    Ok(TcpTask {
        host: String::from(host),
        port: s[(i + 1)..].parse::<u16>().with_context(|| anyhow!("{s:?}"))?,
    })
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
    if let Ok(ip) = IpAddr::from_str(host) {
        return Ok(vec![SocketAddr::new(ip, port)]);
    }

    // The system resolver goes first so that `/etc/hosts`, NSS and mDNS entries keep working.
    if let Ok(addrs) = lookup_host((host, port)).await {
        let addrs: Vec<SocketAddr> = addrs.collect();

        if !addrs.is_empty() {
            return Ok(addrs);
        }
    }

    Ok(DNS_CLIENT
        .query_addrs(host)
        .await
        .with_context(|| anyhow!("{host:?}"))?
        .into_iter()
        .map(|ip| SocketAddr::new(ip, port))
        .collect())
}

async fn wait_tcp_handler(tcp_task: &TcpTask, last_error: &mut Option<anyhow::Error>) {
    loop {
        match host_port_to_socket_addrs(tcp_task.host.as_str(), tcp_task.port).await {
            Ok(addrs) => {
                *last_error = None;

                for addr in addrs {
                    if let Ok(Ok(_)) =
                        time::timeout(CONNECT_TIMEOUT, TcpStream::connect(addr)).await
                    {
                        return;
                    }
                }
            },
            // The service may not be registered in DNS yet, so keep retrying until the timeout.
            Err(error) => *last_error = Some(error),
        }

        sleep(SLEEP_INTERVAL).await;
    }
}

#[inline]
async fn wait_tcp(tcp_task: &TcpTask, timeout: Duration) -> anyhow::Result<()> {
    let mut last_error = None;

    if timeout.is_zero() {
        wait_tcp_handler(tcp_task, &mut last_error).await;

        return Ok(());
    }

    let result = time::timeout(timeout, wait_tcp_handler(tcp_task, &mut last_error)).await;

    if result.is_err() {
        let message =
            format!("Cannot connect to server: {}:{} timeout.", tcp_task.host, tcp_task.port);

        return Err(match last_error {
            Some(error) => error.context(message),
            None => anyhow!(message),
        });
    }

    Ok(())
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

#[cfg(feature = "json")]
async fn load_json_tasks(paths: Vec<PathBuf>) -> anyhow::Result<Vec<Task>> {
    let mut tasks = Vec::new();

    for path in paths {
        let text = fs::read_to_string(path.as_path()).await.with_context(|| {
            anyhow!("{:?} cannot be successfully read.", path.absolutize().unwrap())
        })?;

        let file_tasks: Vec<Task> = serde_json::from_str(text.as_str()).with_context(|| {
            anyhow!("{:?} is not a correct service list file", path.absolutize().unwrap())
        })?;

        tasks.extend(file_tasks);
    }

    Ok(tasks)
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let args = get_args();

    let timeout = Duration::from_secs(args.timeout);

    let mut tcp_tasks = Vec::with_capacity(args.tcp.len());

    for e in args.tcp {
        tcp_tasks.push(parse_tcp_arg(e.as_str())?);
    }

    #[cfg(unix)]
    let mut uds_tasks = Vec::with_capacity(args.uds.len());

    #[cfg(unix)]
    for uds in args.uds {
        uds_tasks.push(UdsTask {
            uds,
        });
    }

    #[cfg(feature = "json")]
    for task in load_json_tasks(args.json).await? {
        match task {
            Task::Tcp(task) => tcp_tasks.push(task),
            #[cfg(unix)]
            Task::Uds(task) => uds_tasks.push(task),
        }
    }

    #[cfg(unix)]
    let task_count = tcp_tasks.len() + uds_tasks.len();

    #[cfg(windows)]
    let task_count = tcp_tasks.len();

    if task_count == 0 {
        eprintln!("Warning: no service to wait for.");

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

                    sender.send(false).await.unwrap();
                },
            }
        });
    }

    // The remaining sender has to be dropped, otherwise a panicking task would leave `recv` pending forever.
    drop(sender);

    for _ in 0..task_count {
        match receiver.recv().await {
            Some(true) => (),
            _ => process::exit(-1),
        }
    }

    exec(args.command)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_tcp_arg_with_host_name() {
        let task = parse_tcp_arg("localhost:27017").unwrap();

        assert_eq!("localhost", task.host);
        assert_eq!(27017, task.port);
    }

    #[test]
    fn parse_tcp_arg_with_ipv4() {
        let task = parse_tcp_arg("127.0.0.1:80").unwrap();

        assert_eq!("127.0.0.1", task.host);
        assert_eq!(80, task.port);
    }

    #[test]
    fn parse_tcp_arg_with_ipv6() {
        let task = parse_tcp_arg("[::1]:8080").unwrap();

        assert_eq!("::1", task.host);
        assert_eq!(8080, task.port);
    }

    #[test]
    fn parse_tcp_arg_without_port() {
        assert!(parse_tcp_arg("localhost").is_err());
    }

    #[test]
    fn parse_tcp_arg_with_invalid_port() {
        assert!(parse_tcp_arg("localhost:http").is_err());
    }
}
