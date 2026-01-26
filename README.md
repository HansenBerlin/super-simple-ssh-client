# Super Simple SSH

Terminal-only SSH client built with `ratatui`. It stores connections securely (master password + encryption), keeps a short history, and supports upload/download via SFTP. After initial setup of connections, ssh into remote host with the press of a key.

## Install

### Debian/Ubuntu (deb)
1. Download the `.deb` from the GitHub release.
2. Install:
   ```bash
   sudo apt install ./super-simple-ssh-client_0.1.0_amd64.deb
   ```
3. Run:
   ```bash
   ss-ssh
   ```

### Fedora/RHEL (rpm)
1. Download the `.rpm` from the GitHub release.
2. Install:
   ```bash
   sudo dnf install ./super-simple-ssh-client-0.1.0-1.x86_64.rpm
   ```
3. Run:
   ```bash
   ss-ssh
   ```

### Arch (pkg.tar.zst)
1. Download the `.pkg.tar.zst` from the GitHub release.
2. Install:
   ```bash
   sudo pacman -U ./super-simple-ssh-client-0.1.0-1-x86_64.pkg.tar.zst
   ```
3. Run:
   ```bash
   ss-ssh
   ```

### Generic Linux (tar.gz)
1. Download the `super-simple-ssh-client-x86_64-unknown-linux-gnu.tar.gz` artifact.
2. Extract and run:
   ```bash
   tar -xzf super-simple-ssh-client-x86_64-unknown-linux-gnu.tar.gz
   ./ss-ssh
   ```

## Usage

### First run
- You will be prompted to create a master password.
- This password encrypts stored connection data.

### Main view
Global commands (see the help header):
- `(t)erminal` open terminal for the selected connected host
- `(u)pload` upload to the selected connected host
- `(d)ownload` download from the selected connected host
- `(o)ptions` change master password
- `(v)iew` toggle header mode (help / logs / off)
- `(q)uit`

Connection list commands (bottom of left panel):
- `(n)ew` add connection
- `(e)dit` edit connection
- `(c)onnect` or `(c)ancel` based on connection state
- `(x)delete`

Navigation:
- `Tab` / `Shift+Tab` or `Up/Down` to move in lists
- `Left/Right` to page through connection history
- `Enter` to activate the selected action in dialogs
- `Esc` to close dialogs

### Connection setup
When creating/editing a connection:
- Pick auth type (password or private key)
- Private key supports optional key password
- You can browse keys with `F2` or pick recent keys with `F3`
- Actions at the bottom: `Test connection` and `Save connection`
- Optionally use a friendly name that will show in the list instead of the hostname
- Connections are saved in the app config directory.

### File transfer
Upload/download is multi-step:
- Select connection
- Select direction (upload or download by pressing `u` or `d` respectively)
- Select source (file or folder)
- Select target (directory only)

Picker controls:
- `Enter` open directory
- `S` select current directory
- `Backspace` go up
- `B` go back one step (target -> source, confirm -> target)
- `Esc` cancel

Transfers show a progress bar; you can hide it with `Enter` and cancel with `Esc`. Progress is also logged.

## How it works
- Connection configs are encrypted using a master password.
- Successful connections are saved and sorted by recent use.
- Open connections are managed in tabs (shown at the top).
- Transfers use SFTP over the existing SSH setup.
- Logs are stored under the app config directory and shown in the UI when enabled.
