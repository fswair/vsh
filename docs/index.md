---
title: Overview
hide:
  - toc
---

<div class="vsh-home" markdown="1">

<section class="vsh-intro">
  <p class="vsh-eyebrow">Transactional filesystem simulation</p>
  <h1 id="__skip">Make the change.<br><span>Before you make it real.</span></h1>
  <p class="vsh-intro-lead">
    Run workspace automation in a virtual snapshot. Review the proposed changes.
    Commit when you're ready. One fast Rust engine, at home in Rust and Python.
  </p>
  <div class="vsh-actions">
    <a class="vsh-button" href="start/">Start building <span aria-hidden="true">↗</span></a>
    <a class="vsh-text-link" href="start/how-it-works/">How VSH works <span aria-hidden="true">→</span></a>
  </div>
</section>

<section class="vsh-example" markdown="1">
<div class="vsh-example-heading"><span>Your first preview</span><span class="vsh-caption">Changes stay virtual</span></div>

=== "Python"

    ```python
    from vsh import ReceiptDetail, Runtime

    runtime = Runtime.open("./project")
    receipt = runtime.preview(
        "vsh_write('/workspace/hello.txt', 'Hello, VSH!')",
        detail=ReceiptDetail.FULL,
    )

    print(receipt.changes)  # Inspect before committing
    ```

=== "Rust"

    ```rust
    use vsh::{ReceiptDetail, RunRequest, Runtime, RuntimeConfig};

    fn main() -> Result<(), vsh::VshError> {
        let config = RuntimeConfig::new("./project");
        let runtime = Runtime::open(config)?;
        let code = "vsh_write('/workspace/hello.txt', 'Hello, VSH!')";
        let request = RunRequest::new(code)
            .with_detail(ReceiptDetail::Full);
        let receipt = runtime.preview(request)?;
        println!("{:?}", receipt.changes);
        Ok(())
    }
    ```

<div class="vsh-example-footer"><span class="vsh-status-dot" aria-hidden="true"></span><span>One virtual file. No user-file changes applied.</span><a href="guides/transactions/">Understand the receipt <span aria-hidden="true">→</span></a></div>
</section>

These `vsh_*` functions are included in VSH. Start with the
[self-contained installation tutorial](start/index.md) for worker setup and a complete
preview → review → commit example.

<section class="vsh-paths" aria-labelledby="choose-your-path">
  <div class="vsh-section-heading"><h2 id="choose-your-path">Find your way in.</h2><p>Same engine. Your workflow.</p></div>
  <div class="vsh-path-grid">
    <a class="vsh-path" href="python/"><span class="vsh-path-number" aria-hidden="true">01</span><span><strong>Python SDK</strong><span>A native engine with a familiar Python API.</span></span><span aria-hidden="true">↗</span></a>
    <a class="vsh-path" href="rust/"><span class="vsh-path-number" aria-hidden="true">02</span><span><strong>Rust SDK</strong><span>Typed control, straight from the crate.</span></span><span aria-hidden="true">↗</span></a>
    <a class="vsh-path" href="integrations/mcp/"><span class="vsh-path-number" aria-hidden="true">03</span><span><strong>MCP &amp; agents</strong><span>One tool for a complete workspace transaction.</span></span><span aria-hidden="true">↗</span></a>
    <a class="vsh-path" href="integrations/monty-tools/"><span class="vsh-path-number" aria-hidden="true">04</span><span><strong>Monty functions</strong><span>Read, search, patch, and compose in one snapshot.</span></span><span aria-hidden="true">↗</span></a>
  </div>
</section>

<section class="vsh-principles" aria-labelledby="every-change-accounted-for">
  <div class="vsh-section-heading"><h2 id="every-change-accounted-for">Every change, accounted for.</h2><p>From an idea to a verified result.</p></div>
  <ol class="vsh-sequence">
    <li><span class="vsh-step">01 / Simulate</span><h3>A real virtual workspace</h3><p>Monty functions and <code>pathlib</code> share one copy-on-write snapshot. Reads see earlier writes.</p></li>
    <li><span class="vsh-step">02 / Inspect</span><h3>Evidence you can review</h3><p>Inspect changed paths and bounded content. Policy and approval bind to the exact transaction.</p></li>
    <li><span class="vsh-step">03 / Commit</span><h3>Revalidated, then applied</h3><p>VSH checks for drift, applies the approved changes, and verifies the result with recovery support.</p></li>
  </ol>
  <a class="vsh-text-link" href="security/">Read the security model <span aria-hidden="true">→</span></a>
</section>

<aside class="vsh-engineering-note">
  <span class="vsh-eyebrow">Built to be measured</span>
  <p>Explore the latency measurements, coverage contract, and architectural decisions behind VSH.</p>
  <div><a href="performance/">Benchmarks <span aria-hidden="true">↗</span></a><a href="coverage/">Test coverage <span aria-hidden="true">↗</span></a><a href="ARCHITECTURE/">Architecture <span aria-hidden="true">↗</span></a></div>
</aside>

VSH is for bounded workspace transformations. Monty has no host mount, subprocess,
network capability, or ambient environment. See the [guarantees](guarantees.md)
for the precise execution and commit contract.

</div>
