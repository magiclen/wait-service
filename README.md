Wait Service
====================

[![CI](https://github.com/magiclen/wait-service/actions/workflows/ci.yml/badge.svg)](https://github.com/magiclen/wait-service/actions/workflows/ci.yml)

Wait Service is a pure rust program to test and wait on the availability of multiple services.

## Help

```
EXAMPLES:
wait-service --tcp localhost:27017 --tcp localhost:27018   -t 5 -- npm start   # Wait for localhost:27017 and localhost:27018 (max 5 seconds) and then run `npm start`
wait-service --tcp localhost:27017 --uds /var/run/app.sock -t 0 -- npm start   # Wait for localhost:27017 and /var/run/app.sock (forever) and then run `npm start`
wait-service --uds /var/run/app.sock --json /path/to/json       -- npm start   # Wait for /var/run/app.sock and other services defined in the json file (max 60 seconds) and then run `npm start`

Usage: wait-service [OPTIONS] -- <COMMAND>...

Arguments:
  <COMMAND>...  Command to execute after service is available

Options:
  -t, --timeout <TIMEOUT>  Set the timeout in seconds, zero for no timeout [default: 60]
      --tcp <TCP>...       Test and wait on the availability of TCP services
      --uds <UDS>...       Test and wait on the availability of UDS services [aliases: unix]
      --json <JSON>...     Test and wait on the availability of TCP or UDS services
  -h, --help               Print help
  -V, --version            Print version
```

## Services

Each `--tcp` service is a `host:port` pair. An IPv6 literal has to be bracketed.

```bash
wait-service --tcp 127.0.0.1:8080 --tcp '[::1]:8080' -- npm start
```

`--tcp`, `--uds` and `--json` all accept multiple values after a single flag, so these two are equivalent.

```bash
wait-service --tcp localhost:27017 --tcp localhost:27018 -- npm start
wait-service --tcp localhost:27017 localhost:27018       -- npm start
```

A host name is resolved with the system resolver first, so `/etc/hosts` entries are honored, and a direct DNS query is only used as a fallback. A service that cannot be resolved or connected to yet is retried every 500 milliseconds until the timeout expires.

## The Config File

With the `--json` option, you can input one or more JSON files to import your TCP / UDS services. The content of each file needs to be a JSON array of objects.

For a TCP service, the object format is

```json
{
    "host": "example.com",
    "port": 443
}
```

For a UDS service, the object format is

```json
{
    "uds": "/path/to/socket_file"
}
```

## License

[MIT](LICENSE)