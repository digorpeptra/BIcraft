# Hosting BIcraft for the web

BIcraft's browser app is the same egui UI as the desktop app, compiled to WebAssembly
(`apps/bicraft-web`, [docs/webui.md](../../docs/webui.md)). Unlike the other Crafting Apps, it is
not shipped as a static site: it reads reports through the viewer API of a BIcraft server, so
the bundle is embedded in the `bicraft` binary and served by it.

| To | Use |
|---|---|
| Look at one file in a local browser | `bicraft view FILE` (loopback only; `--ui egui` for the egui app) |
| Host workspaces for several people | `bicraft serve --root DIR` ([docs/web-deployment.md](../../docs/web-deployment.md)) |
| Run it in a container or on Kubernetes | the image and Helm chart under `deploy/` ([docs/kubernetes.md](../../docs/kubernetes.md)) |

Every release binary embeds the bundle (`cargo xtask webui`, then `cargo xtask dist`;
[docs/releasing.md](../../docs/releasing.md)); `bicraft build-info` says whether a binary has it.
