//! L6: the workspace controller of a Kubernetes deployment of `bicraft-cli serve`
//! (docs/web-deployment.md, "Workspace controller"; docs/kubernetes.md, "The controller").
//!
//! - [`reconcile::Controller`] reconciles `BicraftWorkspace` resources: for each one a
//!   `Deployment` (one replica when awake, none when idle), a `Service`, a `NetworkPolicy` and a
//!   `PersistentVolumeClaim`, written with server-side apply and owned by the workspace, so that
//!   deleting the workspace deletes them (the claim only with `reclaimPolicy: Delete`).
//! - [`gateway::Gateway`] is the HTTP entry point: it authenticates with `bicraft-auth` (the same
//!   three modes as `bicraft-cli serve`), checks the workspace's grants, wakes a workspace at zero
//!   replicas and holds the request until its pod is ready, forwards the request with the checked
//!   identity, and serves the admin API. Idle workspaces go back to zero replicas.
//! - [`kube`] is a minimal Kubernetes API client over `ureq` (no async runtime, no new
//!   dependency) behind the [`kube::Kube`] trait, with an in-memory fake for tests.
//!
//! No engine, no async runtime: plain threads, like the rest of the server side.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]
#![cfg_attr(
    test,
    allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unreachable)
)]

pub mod config;
pub mod gateway;
pub mod kube;
pub mod reconcile;
pub mod render;
pub mod spec;

pub use config::Config;
pub use gateway::{Gateway, GatewayConfig, Upstream};
pub use kube::{FakeKube, HttpKube, Kube, KubeError, Resource};
pub use reconcile::Controller;
