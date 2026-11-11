# Install on Linux without Docker

This runs the Odeion node (`odeiond`) as a system service on a Raspberry Pi, a mini PC or any Debian or Ubuntu
machine. You type a few commands in a terminal on that machine (or over SSH).

Which download:

| Machine | File |
| :-- | :-- |
| PC, mini PC, most NAS (64-bit Intel or AMD) | `pubky-music-node-linux-x86_64.tar.gz` |
| Raspberry Pi 4 or 5 with 64-bit Raspberry Pi OS | `pubky-music-node-linux-arm64.tar.gz` |
| Older Pi or 32-bit Raspberry Pi OS (bookworm) | `pubky-music-node-linux-armv7.tar.gz` |

`uname -m` tells you: `x86_64`, `aarch64` (use arm64) or `armv7l` (use armv7).

## 1. Download and install

Set `ARCH` to `x86_64`, `arm64` or `armv7`, then:

```sh
ARCH=arm64
curl -LO https://github.com/gillohner/pubky-music-node/releases/download/edge/pubky-music-node-linux-$ARCH.tar.gz
curl -LO https://github.com/gillohner/pubky-music-node/releases/download/edge/SHA256SUMS
sha256sum -c --ignore-missing SHA256SUMS      # must print OK
tar xzf pubky-music-node-linux-$ARCH.tar.gz
sudo install -m 755 pubky-music-node-linux-$ARCH/odeiond /usr/local/bin/
sudo ln -sf odeiond /usr/local/bin/pm-node   # the old name, for scripts that still use it
sudo apt install ffmpeg      # optional: smaller copies for phones to download
```

The downloads keep their names from before Odeion (`pubky-music-node-…`), and the folder also
has `pm-node`, the node's old name: the same program. The unpacked folder also has `README.md` and these guides (`docs/guides/`), to read offline.
`edge` is built from the newest code. For the last numbered release, use
`releases/latest/download/` instead of `releases/download/edge/`.

## 2. A user and a config file

The node runs as its own user, in the `audio` group so it can use a DAC on this machine.

```sh
sudo useradd --system --home /var/lib/pubky-music --shell /usr/sbin/nologin --groups audio pubky-music
sudo install -d -o pubky-music -g pubky-music -m 750 /var/lib/pubky-music
sudo install -d -m 755 /etc/pubky-music
TOKEN=$(openssl rand -hex 32)
LAN_IP=$(hostname -I | cut -d' ' -f1)
sudo install -o pubky-music -g pubky-music -m 600 /dev/stdin /etc/pubky-music/node.toml <<EOF
data_dir = "/var/lib/pubky-music"

[api]
listen = "0.0.0.0:7788"
token = "$TOKEN"
EOF
echo "Web page: http://$LAN_IP:7788  token: $TOKEN"
```

Write the token down; it is also in `/etc/pubky-music/node.toml`. The node gives speakers this
machine's address on the way to your router. With a VPN running or several networks that may be
the wrong one: then add `public_url = "http://$LAN_IP:7788"` under `[api]`. Your music folder must be
readable by the `pubky-music` user: `sudo -u pubky-music ls /srv/music` (your folder) shows
whether it is.

## 3. Run it as a service

```sh
sudo tee /etc/systemd/system/odeiond.service >/dev/null <<'EOF'
[Unit]
Description=Odeion node
Wants=network-online.target
After=network-online.target sound.target pubky-music-node.service
Conflicts=pubky-music-node.service

[Service]
User=pubky-music
Group=pubky-music
SupplementaryGroups=audio
ExecStart=/usr/local/bin/odeiond -c /etc/pubky-music/node.toml serve
Environment=RUST_LOG=info
Restart=on-failure
RestartSec=5
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=read-only
PrivateTmp=true
ReadWritePaths=/var/lib/pubky-music

[Install]
WantedBy=multi-user.target
EOF
sudo systemctl daemon-reload
sudo systemctl enable --now odeiond
journalctl -u odeiond -f      # the log; Ctrl+C stops following it
```

The user, the config file and the data folder keep their names from before Odeion
(`pubky-music`, `/etc/pubky-music/node.toml`, `/var/lib/pubky-music`), so an update never moves
your library.

### Installed before Odeion (`pubky-music-node.service`)

That unit keeps working: `pm-node` is still installed, and nothing has to change. To move to the
new unit name, first look at the old unit as it runs, including anything you changed in it (an
extra folder in `ReadWritePaths=` for backups, `Environment=` lines, drop-ins):

```sh
systemctl cat pubky-music-node      # its file, then its drop-ins (pubky-music-node.service.d/)
```

Then install the new program and unit, and carry those changes over: the drop-ins move with it,
and lines you edited in the old unit's own file go into a drop-in of the new one.

```sh
sudo install -m 755 pubky-music-node-linux-$ARCH/odeiond /usr/local/bin/
sudo ln -sf odeiond /usr/local/bin/pm-node
# write /etc/systemd/system/odeiond.service as above, then:
sudo mkdir -p /etc/systemd/system/odeiond.service.d
[ ! -d /etc/systemd/system/pubky-music-node.service.d ] ||
  sudo cp -a /etc/systemd/system/pubky-music-node.service.d/. /etc/systemd/system/odeiond.service.d/
sudo systemctl edit odeiond          # your own lines, e.g. ReadWritePaths=/mnt/backup (added to the unit's)
sudo systemctl daemon-reload
systemctl cat odeiond                # the same ReadWritePaths= and Environment= as before
sudo systemctl disable --now pubky-music-node
sudo systemctl enable --now odeiond
```

Run the node from one unit only: each stops the other, and `odeiond.service` starts after the old
one has stopped, but only one should start at boot. The same user, config and data are used, so the
library and sign-ins are all there. To go back:
`sudo systemctl disable --now odeiond && sudo systemctl enable --now pubky-music-node`.

Open `http://<this machine's address>:7788` (step 2 printed it), open **Use the node’s API token
instead**, paste the token and press **Log in with the token**. Then in **Your library › Node
library**, type your music folder's path (for example `/srv/music`) and press **Add folder**. (Or
list it in the config before the start: `[library]` `folders = ["/srv/music"]` in
`/etc/pubky-music/node.toml`; the node then reads it at every start.) Continue with the [first
steps](overview.md#first-steps-after-installing).

Music inside your home folder works only if the `pubky-music` user may read it; many systems keep
home folders private. A folder such as `/srv/music` is simpler.

**Music on a USB drive:** mount it at a fixed place, so it is there after every restart. Find the
drive with `lsblk -f` (for example `sda1`, with its `UUID`), then:

```sh
sudo mkdir -p /srv/music
echo 'UUID=<the drive's UUID> /srv/music auto ro,nofail 0 2' | sudo tee -a /etc/fstab
sudo mount -a
```

`ro` mounts it read-only, which is all the node needs. Drives formatted for Windows or macOS
(NTFS, exFAT) may need `uid=pubky-music` added after `ro,` so the node can read them.

## A DAC on this machine

A USB DAC or HDMI output on this machine shows up as a speaker by itself. On a desktop system,
PipeWire or PulseAudio may hold the DAC: the node then can't open it. Use a DAC that the desktop
doesn't use, or stop the desktop's sound server for it.

## Updating

Download the new file as in step 1, install `odeiond` again (and the `pm-node` link), then
`sudo systemctl restart odeiond` (`pubky-music-node` on an install from before Odeion). Your library and settings stay in
`/var/lib/pubky-music`. See [Backups and updates](backup-and-update.md).

## With your own Pubky homeserver

The same setup with PostgreSQL and a Pubky homeserver on this machine is in
[docs/deploy.md](../deploy.md#with-its-own-pubky-homeserver-without-docker-bare-metal).

## Building it yourself

```sh
sudo apt install build-essential pkg-config libasound2-dev
curl https://sh.rustup.rs -sSf | sh       # Rust, if you don't have it
git clone https://github.com/gillohner/pubky-music-node.git
cd pubky-music-node
cargo build --release -p pm-node --features alsa      # builds odeiond, and pm-node (its old name)
sudo install -m 755 target/release/odeiond /usr/local/bin/
sudo ln -sf odeiond /usr/local/bin/pm-node
```

Then continue with step 2. The build takes a while and several GB of disk; on a Pi 4 it can take
an hour.
