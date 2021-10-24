Wait Service
====================

[![CI](https://github.com/magiclen/wait-service/actions/workflows/ci.yml/badge.svg)](https://github.com/magiclen/wait-service/actions/workflows/ci.yml)

Wait Service is a pure rust program to test and wait on the availability of a service.

## Help

```
EXAMPLES:
wait-service tcp -h localhost -p 27017 -t 5 -- npm start   # Wait for localhost:27017 (max 5 seconds) and then run `npm start`
wait-service uds -p /var/run/app.sock -t 0 -- npm start    # Wait for /var/run/app.sock (forever) and then run `npm start`

USAGE:
    wait-service [SUBCOMMAND]

FLAGS:
    -h, --help       Prints help information
    -V, --version    Prints version information

SUBCOMMANDS:
    help    Prints this message or the help of the given subcommand(s)
    tcp     Test and wait on the availability of a TCP service
    uds     Test and wait on the availability of a UDS service [aliases: unix]
```

## License

[MIT](LICENSE)