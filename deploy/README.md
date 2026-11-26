# Deploying `doip-server`

The reference relay host is Debian 13 at `176.53.237.193` (root SSH). The service
is **plaintext only** for now: use it for development/staging, not real vehicle
traffic.

## One-time host setup

```sh
export DEBIAN_FRONTEND=noninteractive
apt-get update
apt-get install -y --no-install-recommends \
  build-essential pkg-config git rsync curl ca-certificates

curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
  | sh -s -- -y --profile minimal --default-toolchain stable --no-modify-path
. "$HOME/.cargo/env"
```

The firewall must allow the relay port (the reference host already does):

```sh
ufw allow 13500/tcp
```

## Build and install

From the repository root on the development machine:

```sh
ssh root@176.53.237.193 'mkdir -p /opt/doip-remote/src'
rsync -az --delete \
  --exclude target --exclude .git --exclude '*.log' \
  ./ root@176.53.237.193:/opt/doip-remote/src/

ssh root@176.53.237.193 '
  . "$HOME/.cargo/env" &&
  cd /opt/doip-remote/src &&
  cargo build --release -p doip-server &&
  install -m 0755 target/release/doip-server /usr/local/bin/doip-server
'
```

Create the service user and install the unit:

```sh
ssh root@176.53.237.193 '
  id -u doip >/dev/null 2>&1 ||
    useradd --system --no-create-home --shell /usr/sbin/nologin doip
  install -m 0644 /opt/doip-remote/src/deploy/doip-server.service \
    /etc/systemd/system/doip-server.service
  systemctl daemon-reload
  systemctl enable --now doip-server
'
```

## Verify

```sh
ssh root@176.53.237.193 'systemctl --no-pager status doip-server'
ssh root@176.53.237.193 'ss -ltnp | grep 13500'
ssh root@176.53.237.193 'journalctl -u doip-server -n 20 --no-pager'
```

A successful pairing logs:

```text
waiting for peer session=... role=car
paired session session=... car=... diag=...
tunnel closed session=... car_to_diag=27 diag_to_car=27 elapsed_ms=45
```

## Updating

Repeat the `rsync` + `cargo build --release` + `install` steps, then:

```sh
ssh root@176.53.237.193 'systemctl restart doip-server'
```

The unit uses `Restart=on-failure`; configuration flags can be added to
`ExecStart` in `deploy/doip-server.service` (for example
`--pending-timeout 600`).

## Fast path: install a locally built binary

Building on the VPS is optional. A release binary built on a Linux x86_64 host is
compatible as long as it needs at most `GLIBC_2.34` (the reference host runs
Debian 13 with glibc `2.41`). Check with:

```sh
objdump -T target/release/doip-server | grep -o 'GLIBC_[0-9.]*' | sort -Vu | tail -1
```

Then copy and swap it in atomically, and restart the unit:

```sh
cargo build --release -p doip-server
scp target/release/doip-server root@176.53.237.193:/root/doip-server

ssh root@176.53.237.193 '
  chmod +x /root/doip-server &&
  install -m 0755 /root/doip-server /usr/local/bin/doip-server.new &&
  mv /usr/local/bin/doip-server.new /usr/local/bin/doip-server &&
  systemctl restart doip-server
'
```

The `.new` + `mv` swap avoids replacing the binary while systemd is starting it.
Confirm the copy and the running service:

```sh
sha256sum target/release/doip-server
ssh root@176.53.237.193 '
  sha256sum /usr/local/bin/doip-server
  systemctl is-active doip-server
  ss -ltnp | grep 13500
'
```

Use the systemd unit rather than a second `nohup` process: a second instance
would fail to bind `0.0.0.0:13500`. The binary takes `--listen 0.0.0.0:13500`,
not `--port`.

> Note: the `doip-relay` SSH alias may not exist on every workstation. If
> `ssh doip-relay` fails with `Could not resolve hostname`, use
> `root@176.53.237.193` with the same key.
