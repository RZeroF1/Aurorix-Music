# Publication Policy

Every tracked item in this repository is public by design. Before adding a
file, issue attachment, workflow log, example, or commit message, assume it
will be indexed, copied, and retained indefinitely.

## Appropriate Public Material

- Source code, deterministic tests, sanitized fixtures, and build definitions.
- High-level architecture, stable public API specifications, SDK documents,
  examples, release notes, and user documentation.
- Security design principles that do not disclose real keys, live deployment
  topology, unpatched vulnerabilities, or active incident details.

## Material That Stays Private

- Credentials, API tokens, Provider cookies, private keys, recovery codes, and
  production configuration.
- Production hostnames, IP addresses, SSH aliases, network diagrams, internal
  backup locations, and monitoring identifiers.
- Private threat reports, vulnerability reproduction details before a fix,
  incident records, account data, and private test data.
- Working architecture drafts, risk registers, recovery playbooks, and design
  notes that maintainers have not explicitly approved for publication.

## Process

1. Start new design work in the maintainer's private planning location.
2. Review it for stability, security disclosure, licensing, and contributor
   usefulness.
3. Create a sanitized public document rather than publishing a working draft
   unchanged.
4. Link public contracts to released code and state their compatibility scope.

The ignored `private/` directory is a local publication guard only. A separate
private repository and a proper secret manager remain the right places for
important internal material and credentials.
