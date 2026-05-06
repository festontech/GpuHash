# Ethics & Rules of Engagement

This project is presented and gated as a **defensive, educational** tool. The intended use cases are:

- Auditing the strength of password hashes on systems **you own or are explicitly authorized to test**.
- Benchmarking GPU compute throughput on commodity hardware.
- Teaching parallel programming, WGSL, async Rust, and full-stack desktop architecture.

It is **not** a tool for cracking hashes you do not own.

---

## Hard rules in code

These are enforced or surfaced by the codebase itself, not just policy:

1. **No bundled wordlists or breach corpora.** The `examples/` directory ships only a tiny synthetic word list and synthetic MD5 hashes of those words. The user supplies their own input for any real audit.
2. **Required acknowledgement flag.** The CLI requires `--i-own-these-hashes` for any attack mode. Trivially bypassable, but the friction documents intent.
3. **Auditing-first user-facing language.** UI labels and CLI summaries say *Audit*, *Audit Strength*, *Run Audit* — never *Crack* / *Attack* / *Hack*. Internal type names like `AttackRunner` are technical jargon and live in the engine; they should not appear in user-facing strings.
4. **Default to local-only.** All persistence (sessions, results) lives under the user's local AppData. Nothing leaves the machine. Distributed mode is **out of scope** — one laptop only.
5. **No leak-ingestion features.** Refuse to add integrations with breach dump sites, password-manager exports, or anything aimed at acquiring hashes rather than auditing them.

## Hard rules in conduct

These belong in the project README and the final report:

1. **Disclaimer at the top of README.md.** "This is an educational password-auditing and GPU compute benchmark tool. Use against systems or data you do not have explicit authorization to test is illegal in most jurisdictions and is not supported by this project."
2. **Demo etiquette.** Demos use synthetic hashes the presenter generated themselves, never anything sourced from third parties.
3. **Final report framing.** Three pillars: (a) GPU compute education, (b) password auditing for defenders, (c) honest performance comparison with industry tools. Not "I built a Hashcat clone."

## What this project will not add

Even if requested:

- Integrations with leaked-hash dump sites or breach databases.
- Network sniffers, ARP poisoning, or anything aimed at *capturing* hashes from systems.
- Pre-baked rule sets curated to maximize success against real-world breached passwords.
- Stealth, anti-detection, or anti-forensics features.

If the project ever drifts toward those, that drift is a bug — fix it by removing the offending code, not by adding more guardrails on top.

## Legal context (Netherlands / EU, where the student is studying)

(Non-legal advice; treat this as a starting point.)

- Computer-misuse legislation in NL (Wet computercriminaliteit III) and the EU Cybercrime directive criminalize accessing computer systems or data without authorization.
- Auditing your **own** hashes (e.g. credentials in your own systems) is generally fine. Auditing an employer's hashes requires written authorization (typical penetration-testing contract).
- Distributing tools is legal in the abstract; the framing matters: educational/auditing tools are uncontroversial, "cracking suites" are not.

The single best protection is to keep this project unambiguously a **defensive auditing and educational benchmarking** tool in every artifact: code, UI, documentation, and presentation.

---

## License

This project is offered under MIT OR Apache-2.0. Note that license text doesn't override misuse — the license permits use, it doesn't permit illegal use.
