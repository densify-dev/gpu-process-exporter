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

## Developer Certificate of Origin

This project follows the [Developer Certificate of Origin 1.1](https://developercertificate.org/). Every human-authored commit in a pull request must include a `Signed-off-by` trailer matching the commit author's name and email.

Use Git's sign-off option when creating or updating commits:

```bash
git commit --signoff
git commit --amend --signoff
git rebase --signoff origin/main
git push --force-with-lease
```

Use `git commit --signoff` for a new commit. Use `git commit --amend --signoff` to repair the latest commit. After adding sign-offs with `git rebase --signoff origin/main`, update the remote branch with `git push --force-with-lease`.
