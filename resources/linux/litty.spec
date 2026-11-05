# Preserve the already released executable byte-for-byte.
%global debug_package %{nil}
%global __os_install_post %{nil}
%global _build_id_links none
# There is no source changelog for this prebuilt-asset wrapper.
%global source_date_epoch_from_changelog 0

Name: litty
Version: @VERSION@
Release: 1
Summary: GPU-accelerated terminal emulator
License: MIT
URL: https://github.com/Matrix-I/litty-terminal
Source0: litty
Source1: litty.desktop
Source2: icon.png
Source3: README.rpm
BuildArch: x86_64
BuildRequires: desktop-file-utils
Requires: glibc >= 2.34
Requires: libgcc_s.so.1()(64bit)
Requires: libfontconfig.so.1()(64bit)
Requires: libxkbcommon.so.0()(64bit)
Requires: libxkbcommon-x11.so.0()(64bit)
Requires: libvulkan.so.1()(64bit)
Requires: libwayland-client.so.0()(64bit)
Requires: libwayland-cursor.so.0()(64bit)
Requires: libwayland-egl.so.1()(64bit)
Requires: libX11.so.6()(64bit)
Requires: libX11-xcb.so.1()(64bit)
Requires: libxcb.so.1()(64bit)
Requires: libXcursor.so.1()(64bit)
Requires: libXi.so.6()(64bit)
Requires: libXrandr.so.2()(64bit)
Recommends: mesa-vulkan-drivers
Recommends: dejavu-sans-mono-fonts
Recommends: xdg-utils
Recommends: wl-clipboard
Recommends: xclip

%description
Litty renders terminal sessions with Vulkan and supports tabs, splits,
search, shell integration, and an applications-menu launcher.

%prep
cp -p %{SOURCE3} README.rpm

%build
# The executable is the original release asset; no compilation is needed.

%install
install -D -m 755 %{SOURCE0} %{buildroot}%{_bindir}/litty
install -D -m 644 %{SOURCE1} %{buildroot}%{_datadir}/applications/litty.desktop
install -D -m 644 %{SOURCE2} %{buildroot}%{_datadir}/icons/hicolor/512x512/apps/litty.png

%check
desktop-file-validate %{buildroot}%{_datadir}/applications/litty.desktop

%files
%defattr(-,root,root,-)
%{_bindir}/litty
%{_datadir}/applications/litty.desktop
%{_datadir}/icons/hicolor/512x512/apps/litty.png
%doc README.rpm
