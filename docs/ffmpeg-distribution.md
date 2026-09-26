# FFmpeg build input and provenance

Checked 2026-09-27 for the Windows x64 native v0.2 build. This record pins the FFmpeg command-line build used for local validation; it is not a complete third-party source bundle or legal determination.

## v0.2 source-only publication

The public GitHub push is source-only. Keep `ffmpeg.exe`, `ffprobe.exe`, `ffplay.exe`, and downloaded FFmpeg archives out of the public Git tree. The local installer prepared for the project owner may use the pinned build below. The public source repository does not itself redistribute those binaries.

If a future public installer or release includes FFmpeg binaries, do not treat the upstream FFmpeg source tarball or the package's `LICENSE.txt` as complete source provenance: this static build also contains external libraries. Include source and license information for the exact dependencies enabled in the delivered executables, or replace this broad prebuilt with a narrower reproducible build and its complete source materials.

## Recommended local Windows build

Use BtbN's static `win64-lgpl` build, pinned to the dated release tag rather than the floating `latest` alias:

| Item | Pinned value |
|---|---|
| Release | [BtbN Auto-Build 2026-09-26 13:03](https://github.com/BtbN/FFmpeg-Builds/releases/tag/autobuild-2026-09-26-13-03) |
| Archive | [`ffmpeg-n8.1.3-win64-lgpl-8.1.zip`](https://github.com/BtbN/FFmpeg-Builds/releases/download/autobuild-2026-09-26-13-03/ffmpeg-n8.1.3-win64-lgpl-8.1.zip) |
| Archive size | 170,607,374 bytes |
| SHA-256 | `933b9625fb4b0dc2e1e96cf20fb54b94ed24ba561858418de29531fb7c88ad74` |
| Upstream FFmpeg source tag | [`n8.1.3`](https://github.com/FFmpeg/FFmpeg/tree/n8.1.3), commit `1041abdc962f4cc4f394aa8de9dc5236c0c3b9e7` |
| Builder source tag | [`autobuild-2026-09-26-13-03`](https://github.com/BtbN/FFmpeg-Builds/tree/autobuild-2026-09-26-13-03), commit `58cc05f33c20e3ead0ce876b72531ab482d0f981` |
| FFmpeg source archive | [FFmpeg `n8.1.3` source tar.gz](https://codeload.github.com/FFmpeg/FFmpeg/tar.gz/refs/tags/n8.1.3), SHA-256 `09f990289327d3ebfedf4fc15cae3f884a65dab7a5aac7bbb5b01ff83900efd9` |
| Official checksum manifest | [BtbN `checksums.sha256`](https://github.com/BtbN/FFmpeg-Builds/releases/download/autobuild-2026-09-26-13-03/checksums.sha256) |

BtbN describes its `lgpl` variant as excluding GPL-only libraries, prominently libx264 and libx265. Its pinned `variants/win64-lgpl.sh` selects the static Windows package; `variants/defaults-lgpl.sh` sets `--enable-version3 --disable-debug` and `LICENSE_FILE=COPYING.LGPLv3`. The build script clones FFmpeg from the `release/8.1` branch and copies that FFmpeg license text into the ZIP as `LICENSE.txt`. FFmpeg's own overview says the project is LGPL by default and that enabling its optional GPL components makes the FFmpeg build GPL; see the [official FFmpeg license notes](https://ffmpeg.org/legal.html).

The release asset name identifies FFmpeg `n8.1.3`. The dated release was published at 2026-09-26 13:03 UTC; the two commits now ahead of the `n8.1.3` tag on `release/8.1` are dated 14:46 and 16:05 UTC that day. This supports `n8.1.3` as the upstream branch tip for this build window, but the release does not provide a signed build attestation or embedded dependency-source bundle. When consuming the archive, record `ffmpeg -version`, `ffmpeg -buildconf`, `ffmpeg -L`, and `ffprobe -version` alongside the hash.

The ZIP contains `ffmpeg.exe` (134,087,168 bytes), `ffprobe.exe` (133,881,856 bytes), an unused `ffplay.exe` (137,609,216 bytes), documentation, presets, and `LICENSE.txt`. The two needed executables each exceed 100 MiB, which GitHub blocks in ordinary Git repositories ([large-file limits](https://docs.github.com/en/repositories/working-with-files/managing-large-files/about-large-files-on-github)). Extract only the needed tools and FFmpeg license into a local installer staging area; keep them out of the public source tree.

## Why not the Gyan essentials archive

Gyan's Windows builds page currently labels all builds static and GPLv3. Its smaller `essentials` variant still contains libx264 and libx265; as checked on this date, the current 9.0.2 archive is 34 MB, and the previous 8.1.2 archive is 32 MB. It is a useful compact build but it is not the LGPL alternative. The installed Gyan 8.1.1 full build (`--enable-gpl --enable-version3`) is likewise a GPL build. Gyan links the FFmpeg source commit for each release, but the build page does not provide the same pinned external-dependency recipe set as BtbN's builder repository. See [Gyan's build and library list](https://www.gyan.dev/ffmpeg/builds/).

## Source availability boundary

The BtbN ZIP includes the FFmpeg `COPYING.LGPLv3` text, but not the FFmpeg source tree, BtbN build recipes, or sources for the statically included third-party libraries. The pinned [BtbN builder source](https://github.com/BtbN/FFmpeg-Builds/tree/autobuild-2026-09-26-13-03) contains `scripts.d` package recipes and patches that identify dependency source locations and versions; those recipes are not the dependencies' source archives. The FFmpeg source archive above covers FFmpeg itself only.

FFmpeg's official legal notes describe their checklist as guidance for linking against FFmpeg libraries, including distribution of corresponding source and build information. This application invokes FFmpeg as separate command-line processes rather than linking to libav DLLs, so this page records binary provenance and does not claim a legal conclusion for other kinds of redistribution. Any future public binary installer needs a separate dependency-source and notice review against the actual executables and their `-buildconf` output.
