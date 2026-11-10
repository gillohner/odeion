# Install an Odeion box from a USB stick

The Odeion installer turns a spare PC or mini PC (64-bit Intel or AMD) into an Odeion box: Debian Linux, the Odeion
node and, by default, your own Pubky homeserver. You answer a few questions; nothing else needs a terminal.

**The installer erases the disk you install to.** Use a machine, or a disk, with nothing on it you want to keep.

You need:
- a USB stick of 2 GB or more (it is overwritten too);
- the PC, with a network cable (or Wi-Fi), a screen and a keyboard for the install;
- 20 GB of disk or more, plus room for your music if it lives on that machine.

## 1. Download and check

From the [releases](https://github.com/gillohner/pubky-music-node/releases), take `odeion-<version>-amd64.iso` and
its `.sha256` file. To try the newest build of `main` instead, use the `edge` release, which has
`odeion-edge-amd64.iso`.

```sh
curl -LO https://github.com/gillohner/pubky-music-node/releases/download/edge/odeion-edge-amd64.iso
curl -LO https://github.com/gillohner/pubky-music-node/releases/download/edge/odeion-edge-amd64.iso.sha256
sha256sum -c odeion-edge-amd64.iso.sha256      # must print OK
```

On Windows: `certutil -hashfile odeion-edge-amd64.iso SHA256` prints the checksum. Compare it with the `.sha256` file.

## 2. Put it on the USB stick

- **The easy way:** [balenaEtcher](https://etcher.balena.io): choose the ISO, choose the stick, Flash.
- **On Linux or macOS:** `dd`. Find the stick's name first (`lsblk` on Linux, `diskutil list` on macOS). Writing to
  the wrong disk erases it.

  ```sh
  sudo dd if=odeion-edge-amd64.iso of=/dev/sdX bs=4M status=progress conv=fsync
  ```

## 3. Install

1. Plug in the stick and start the PC from it. Its boot menu is usually opened with F12, F11, F10 or Esc right after
   power-on. UEFI and older BIOS both work. Secure Boot can stay on.
2. The installer starts by itself and asks:
   - **What to run:**
     - **Odeion with its own homeserver** (recommended): the Odeion node and a Pubky homeserver on this machine,
       for your sign-in and your music's backups.
     - **Odeion only:** just the node. You sign in with a Pubky homeserver you already have.
   - **Hostname:** the box's name on your network. Keep `odeion` unless you have more than one.
   - **Wi-Fi network and password:** only when no network cable is plugged in.
   - **A password for the `odeion` user:** for the console only, if you ever need to log in on the box itself.
     Odeion has no remote login (no SSH).
   - **The disk to install to:** then it asks once more before it erases it.
3. It downloads and installs the rest (10 to 30 minutes, depending on your connection) and restarts. Take out the USB
   stick when it says so.
4. On its first start the box downloads Odeion (a few hundred MB). The screen says where to go next.

## 4. Set it up

Open **http://odeion.local:7788** in a browser on the same network. If your device doesn't find `odeion.local`, use
the address the box's screen shows, like `http://192.168.1.20:7788`. The first time, log in with the box's API token:
on the box's console, `sudo grep PM_TOKEN /opt/odeion/.env` shows it; on the Log in page open **Use the node’s API
token instead**, paste it and press **Log in with the token**. The web setup does the rest: create your Pubky account
on the box (or sign in with Pubky Ring), then add your music folders and network shares. From then on you log in with
Pubky Ring.

## What runs on the box

- **Security updates** for Debian install by themselves.
- **Backups** (Odeion with its own homeserver): `/opt/odeion/backup.sh` runs every night into
  `/var/backups/odeion` and keeps the newest 7. It saves the node's library, settings and sign-ins, the homeserver
  and its users' files, never the music. Copy that folder somewhere else from time to time. Odeion only has no
  nightly backup on the box; use the node's own backup in Settings.
- **Firewall:**
  - From anywhere: the node (7788) and, with the homeserver, its Pubky port (6287).
  - From your home network (private addresses): also what speakers need (UPnP, AirPlay, Squeezebox, Snapcast).
  - Nothing else listens on the box: there's no SSH.
- **Watchdog:** a box that hangs restarts by itself.
- **Music:** add a network share (your NAS) or a folder in the web setup. A disk you plug into the box goes in
  `/srv/music`, from the console.
- **A homeserver published through a tunnel:** when its public address isn't the box's own, the box still reaches
  its homeserver directly.

## Updating Odeion

Log in on the box's console as `odeion`, with the password you chose. Odeion lives in `/opt/odeion`; its version is
`PM_TAG` in `/opt/odeion/.env`. A release ISO sets it to that release, the edge ISO to `edge`.

```sh
sudo sed -i 's/^PM_TAG=.*/PM_TAG=0.3.0/' /opt/odeion/.env    # the version you want, or edge
sudo odeion-compose pull
sudo odeion-compose up -d
```

With its own homeserver, the homeserver and its database stay on the versions the ISO came with. A newer Odeion
release says when they need updating too; take `deploy/bundle/docker-compose.yml` from that release into
`/opt/odeion`, then run the same two commands. Back up first (`sudo /opt/odeion/backup.sh`). To stop Odeion:
`sudo odeion-compose stop`.

## Notes

- The installer is Debian 13's own installer (debian-installer), with Odeion's answers filled in. A Raspberry Pi
  image is planned; until then, use [Install on Linux without Docker](install-linux.md) or
  [Install with Docker](install-docker.md) on a Pi.
- CI installs every installer change in a virtual machine, both choices, and so does every release before its
  ISO is attached. The edge ISO is built from `main` without that test. Tell us how it goes on real hardware.
