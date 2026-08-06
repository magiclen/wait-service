mod cli;

#[cfg(any(unix, feature = "json"))]
use std::{borrow::Cow, path::PathBuf};
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
    task::JoinSet,
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

    #[cfg(not(unix))]
    let client = DNSClient::new(dns_servers);

    client
});

#[cfg_attr(feature = "json", derive(Deserialize))]
#[cfg_attr(feature = "json", serde(deny_unknown_fields))]
#[derive(Debug)]
struct TcpTask {
    host: String,
    port: u16,
}

impl TcpTask {
    fn new(host: String, port: u16) -> anyhow::Result<Self> {
        let task = Self {
            host,
            port,
        };

        task.validate()?;

        Ok(task)
    }

    fn validate(&self) -> anyhow::Result<()> {
        if self.host.is_empty() {
            return Err(anyhow!("A TCP service needs to have a host."));
        }

        Ok(())
    }
}

#[cfg(unix)]
#[cfg_attr(feature = "json", derive(Deserialize))]
#[cfg_attr(feature = "json", serde(deny_unknown_fields))]
#[derive(Debug)]
struct UdsTask {
    uds: PathBuf,
}

#[cfg(unix)]
impl UdsTask {
    fn new(uds: PathBuf) -> anyhow::Result<Self> {
        let task = Self {
            uds,
        };

        task.validate()?;

        Ok(task)
    }

    fn validate(&self) -> anyhow::Result<()> {
        if self.uds.as_os_str().is_empty() {
            return Err(anyhow!("A UDS service needs to have a path."));
        }

        Ok(())
    }
}

#[cfg_attr(feature = "json", derive(Deserialize))]
#[cfg_attr(feature = "json", serde(untagged))]
#[derive(Debug)]
enum Task {
    Tcp(TcpTask),
    #[cfg(unix)]
    Uds(UdsTask),
}

impl Task {
    /// Tasks built by `serde` skip the constructors, so they have to be checked afterwards.
    #[cfg(feature = "json")]
    fn validate(&self) -> anyhow::Result<()> {
        match self {
            Self::Tcp(task) => task.validate(),
            #[cfg(unix)]
            Self::Uds(task) => task.validate(),
        }
    }

    async fn wait(self, timeout: Duration) -> anyhow::Result<()> {
        match self {
            Self::Tcp(task) => wait_tcp(&task, timeout).await,
            #[cfg(unix)]
            Self::Uds(task) => wait_uds(&task, timeout).await,
        }
    }
}

/// Parses a `host:port` argument. An IPv6 literal has to be bracketed, e.g. `[::1]:8080`.
fn parse_tcp_arg(s: &str) -> anyhow::Result<TcpTask> {
    if let Ok(addr) = SocketAddr::from_str(s) {
        return TcpTask::new(addr.ip().to_string(), addr.port());
    }

    let i = s.rfind(':').ok_or_else(|| anyhow!("{s:?} needs to have a port!"))?;

    let raw_host = &s[..i];
    let host = raw_host.strip_prefix('[').and_then(|e| e.strip_suffix(']')).unwrap_or(raw_host);

    TcpTask::new(String::from(host), s[(i + 1)..].parse::<u16>().with_context(|| anyhow!("{s:?}"))?)
        .with_context(|| anyhow!("{s:?}"))
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

    #[cfg(not(unix))]
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

    let addrs: Vec<SocketAddr> = DNS_CLIENT
        .query_addrs(host)
        .await
        .with_context(|| anyhow!("{host:?}"))?
        .into_iter()
        .map(|ip| SocketAddr::new(ip, port))
        .collect();

    // A DNS query for a name without any A/AAAA record succeeds with an empty answer.
    if addrs.is_empty() {
        return Err(anyhow!("{host:?} cannot be resolved to any address."));
    }

    Ok(addrs)
}

async fn wait_tcp_handler(tcp_task: &TcpTask, last_error: &mut Option<anyhow::Error>) {
    loop {
        match host_port_to_socket_addrs(tcp_task.host.as_str(), tcp_task.port).await {
            Ok(addrs) => {
                *last_error = None;

                let mut attempts = JoinSet::new();

                for addr in addrs {
                    attempts.spawn(async move {
                        time::timeout(CONNECT_TIMEOUT, TcpStream::connect(addr)).await
                    });
                }

                while let Some(result) = attempts.join_next().await {
                    if matches!(result, Ok(Ok(Ok(_)))) {
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
                uds_task.uds.absolutize().unwrap_or(Cow::Borrowed(uds_task.uds.as_path()))
            )
        })?;
    }

    Ok(())
}

#[cfg(feature = "json")]
async fn load_json_tasks(paths: Vec<PathBuf>) -> anyhow::Result<Vec<Task>> {
    let mut tasks = Vec::new();

    for path in paths {
        let absolute_path = path.absolutize().unwrap_or(Cow::Borrowed(path.as_path()));

        let text = fs::read_to_string(path.as_path())
            .await
            .with_context(|| anyhow!("{absolute_path:?} cannot be successfully read."))?;

        let file_tasks: Vec<Task> = serde_json::from_str(text.as_str())
            .with_context(|| anyhow!("{absolute_path:?} is not a correct service list file"))?;

        for task in &file_tasks {
            task.validate()
                .with_context(|| anyhow!("{absolute_path:?} is not a correct service list file"))?;
        }

        tasks.extend(file_tasks);
    }

    Ok(tasks)
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let args = get_args();

    let timeout = Duration::from_secs(args.timeout);

    let mut tasks = Vec::with_capacity(args.tcp.len());

    for e in args.tcp {
        tasks.push(Task::Tcp(parse_tcp_arg(e.as_str())?));
    }

    #[cfg(unix)]
    for uds in args.uds {
        tasks.push(Task::Uds(UdsTask::new(uds)?));
    }

    #[cfg(feature = "json")]
    tasks.extend(load_json_tasks(args.json).await?);

    if tasks.is_empty() {
        eprintln!("Warning: no service to wait for.");

        return exec(args.command);
    }

    let mut waits = JoinSet::new();

    for task in tasks {
        waits.spawn(task.wait(timeout));
    }

    while let Some(result) = waits.join_next().await {
        // A task can only fail to be joined by panicking, which is a failure as well.
        if let Err(error) = result.map_err(anyhow::Error::from).and_then(|result| result) {
            eprintln!("{error:?}");

            process::exit(-1);
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
    fn parse_tcp_arg_without_host() {
        assert!(parse_tcp_arg(":8080").is_err());
    }

    #[test]
    fn parse_tcp_arg_with_invalid_port() {
        assert!(parse_tcp_arg("localhost:http").is_err());
    }
}
