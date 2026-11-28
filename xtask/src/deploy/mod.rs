//! `cargo xtask deploy check|smoke|kind-smoke`: the Kubernetes packaging of `bicraft-cli serve`
//! (deploy/, docs/kubernetes.md).

mod check;
pub(crate) mod client;
mod controller_smoke;
mod kind;
mod smoke;

const USAGE: &str = "\
usage: cargo xtask deploy <check|smoke|kind-smoke|controller-smoke> [ARGS]

  check [--binary BICRAFT]
               static checks of deploy/ (YAML, drift against the code, security invariants,
               sizing rules, the Dockerfile); also run by `cargo xtask ci` as a unit test;
               with --binary, also that this release binary embeds the browser app
  smoke        `bicraft-cli serve` started as the container starts it, without a container, and
               checked live over MCP Streamable HTTP, then drained (`smoke --help`)
  kind-smoke   the image, a kind cluster, the Helm chart and the same live checks; needs docker,
               kind, helm and kubectl (`kind-smoke --help`)
  controller-smoke
               the workspace controller in a kind cluster: API, wake on request, idle stop,
               NetworkPolicy, deletion and volume policy (`controller-smoke --help`)
";

pub fn cmd(args: &[String]) -> Result<(), String> {
    let rest = args.get(1..).unwrap_or_default();
    match args.first().map(String::as_str) {
        Some("check") => {
            // `--binary PATH`: a release binary to check too (it must embed the browser app)
            match rest {
                [] => {}
                [flag, bin] if flag == "--binary" => {
                    let bin = std::path::Path::new(bin);
                    if !crate::webui::binary_embeds(bin)? {
                        return Err(format!(
                            "deploy check: {} has no browser app (docs/webui.md: release binaries embed it)",
                            bin.display()
                        ));
                    }
                    println!("deploy check: {} embeds the browser app", bin.display());
                }
                _ => return Err(format!("deploy check: unknown arguments\n\n{USAGE}")),
            }
            let root = crate::root();
            match check::run(&root) {
                Ok(s) => {
                    println!(
                        "deploy check: ok: {} YAML files ({} documents), {} templates, {} .Values references, \
                         {} variables and {} serve flags known to the code",
                        s.yaml_files,
                        s.yaml_docs,
                        s.templates,
                        s.values_refs,
                        s.env_vars.len(),
                        s.flags.len()
                    );
                    println!(
                        "  variables: {}",
                        s.env_vars.into_iter().collect::<Vec<_>>().join(" ")
                    );
                    println!("  flags: {}", s.flags.into_iter().collect::<Vec<_>>().join(" "));
                    Ok(())
                }
                Err(errors) => {
                    for e in &errors {
                        println!("  - {e}");
                    }
                    Err(format!("deploy check: {} problem(s)", errors.len()))
                }
            }
        }
        Some("smoke") => smoke::cmd(rest),
        Some("kind-smoke") => kind::cmd(rest),
        Some("controller-smoke") => controller_smoke::cmd(rest),
        Some("-h" | "--help" | "help") | None => {
            print!("{USAGE}");
            Ok(())
        }
        Some(other) => Err(format!("deploy: unknown command `{other}`\n\n{USAGE}")),
    }
}
