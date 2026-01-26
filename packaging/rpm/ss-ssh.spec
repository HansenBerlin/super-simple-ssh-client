Name:           super-simple-ssh-client
Version:        @VERSION@
Release:        1%{?dist}
Summary:        Super simple SSH client
License:        MIT
URL:            https://github.com/HansenBerlin/super-simple-ssh-client
Source0:        %{name}-%{version}.tar.gz
BuildRequires:  cargo
BuildRequires:  rust
BuildRequires:  gcc
BuildRequires:  make
BuildRequires:  openssl-devel
BuildRequires:  libssh2-devel
BuildRequires:  zlib-devel
BuildRequires:  pkgconf-pkg-config
Requires:       openssl
Requires:       libssh2
Requires:       zlib

%description
Terminal SSH client with a ratatui interface.

%prep
%autosetup -n %{name}-%{version}

%build
cargo build --release --locked

%install
install -Dm755 target/release/ss-ssh %{buildroot}/usr/bin/ss-ssh
install -Dm644 packaging/ss-ssh.desktop %{buildroot}/usr/share/applications/ss-ssh.desktop

%files
/usr/bin/ss-ssh
/usr/share/applications/ss-ssh.desktop

%changelog
* Thu Jan 01 1970 hansen_docked_in <hansdrum@proton.me> - %{version}-1
- Initial package
