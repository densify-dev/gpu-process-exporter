# Contributing

Thanks for taking the time to contribute.

## Setup

```bash
go mod download
go test ./...
```

## Formatting and tests

Before opening a pull request, run:

```bash
gofmt -w cmd pkg
go test ./...
go test -race ./...
go build ./...
make lint
```

## Pull requests

- Keep changes small and focused.
- Explain why the change is needed.
- Add or update tests when behavior changes.
- Do not include secrets, generated scan reports, private issue links, or private migration notes.

## License and contributions

This project uses Apache License 2.0. Unless you state otherwise, contributions submitted to this repository are licensed under Apache License 2.0.

This project does not require DCO sign-off.
