Wait Service
====================

[![CI](https://github.com/magiclen/wait-service/actions/workflows/ci.yml/badge.svg)](https://github.com/magiclen/wait-service/actions/workflows/ci.yml)

Wait Service is a pure rust program to test and wait on the availability of multiple services.

## Help

```
EXAMPLES:
wait-service --tcp localhost:27017 --tcp localhost:27018   -t 5 -- npm start  # Wait for localhost:27017 and localhost:27018 (max 5 seconds) and then run `npm start`
wait-service --tcp localhost:27017 --uds /var/run/app.sock -t 0 -- npm start  # Wait for localhost:27017 and /var/run/app.sock (forever) and then run `npm start`
wait-service --uds /var/run/app.sock --json /path/to/json       -- npm start  # Wait for /var/run/app.sock and other services defined in the json file (max 60 seconds) and then run `npm start`

USAGE:
    wait-service [OPTIONS] <COMMAND>...

ARGS:
    <COMMAND>...    Command to execute after service is available

OPTIONS:
    -t, --timeout <TIMEOUT>    Set the timeout in seconds, zero for no timeout [default: 60]
        --tcp <TCP>...         Test and wait on the availability of TCP services
        --uds <UDS>...         Test and wait on the availability of UDS services [aliases: unix]
    -h, --help                 Print help information
    -V, --version              Print version information
```

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