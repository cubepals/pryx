# Native tools image

The immutable base is Python 3.12.14 on Debian bookworm, pinned by digest in
[Dockerfile](Dockerfile). Python uses the PSF licence; Debian retains each installed
package's notices under `/usr/share/doc`. [The Debian package review summary](../../docs/checks/measurements/k18-debian-licences.json) records the review scope; full package versions, hashes, licences and source URLs remain in the image and CI artifacts.

The next layer installs unmodified external programs: OpenSSH 9.2p1 (BSD/ISC/MIT),
FUSE 3 (GPL/LGPL), Xvfb (X11/MIT), JWM (MIT), x11vnc (GPL-2.0), xdotool (BSD), and
scrot (MIT). It retains their full copyright files. No copyleft implementation is
copied into this repository. Rebuilds verify every archive against
[debian.lock.json](debian.lock.json), including optional build-cache entries.

The same layer adds Mountpoint S3 1.24.0 (Apache-2.0 with bundled third-party
notices), rclone 1.76.0 (MIT with vendor notices), and noVNC 1.7.0 (MPL-2.0 with
BSD/OFL and asset notices). [artifacts.json](artifacts.json) pins their hashes and
source URLs. Full external-program notices are installed under
`/opt/pryx/native-tools/notices`; [rclone's module review summary](../../docs/checks/measurements/k18-rclone-modules.json)
records the review scope; resolved versions remain in the pinned source and CI artifacts. The rclone test variant removes the Mountpoint
launcher so acceptance exercises the real fallback rather than a directory mirror.

The final additions are the hash-locked desktop Python dependencies described in
[the desktop README](../desktop/README.md) and pryx's FSL-1.1-ALv2 launch wrappers.
See [native-tools.md](../../docs/native-tools.md) for the explicit host policy;
this image grants no host capability or isolation exception by itself.
