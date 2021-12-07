# rust-terminal


## Prerequisites

### Clone the source code

Before compiling the project, you'll have to first clone the source code:

```sh
https://github.com/taonyzeng/rust-terminal.git
cd rust-terminal
```

### Install the Rust compiler with `rustup`

1. Install [`rustup.rs`](https://rustup.rs/).

3. To make sure you have the right Rust compiler installed, run

   ```sh
   rustup override set stable
   rustup update stable
   ```

### Dependencies

These are the minimum dependencies required to build Alacritty, please note
that with some setups additional dependencies might be desired.

If you're running Wayland with an Nvidia GPU, you'll likely want the EGL
drivers installed too (these are called `libegl1-mesa-dev` on Ubuntu).

#### Debian/Ubuntu

If you'd like to build a local version manually, you need a few extra libraries
to build rust-terminal. Here's an apt command that should install all of them. If
something is still found to be missing, please open an issue.

```sh
apt-get install cmake pkg-config libfreetype6-dev libfontconfig1-dev libxcb-xfixes0-dev libxkbcommon-dev python3
```

