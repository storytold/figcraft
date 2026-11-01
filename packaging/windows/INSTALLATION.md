# Windows installation policy checks

New installations default to the current account under `Programs\ArtCraft\<App>`.
The first setup page offers installation for everyone. Scope is selected before duplicate
checks and directory costing. An update uses the existing context's remembered exact folder;
`INSTALLFOLDER` supplied explicitly is never replaced. It is a secure MSI property so the
chosen folder reaches the elevated installer. Silent machine updates pass `ALLUSERS=1`.
A cross-context copy is refused; changing context means uninstalling the old copy first.

App registration follows the selected context. Existing optional shortcuts and file-handler
choices stay app-owned. No new cleanup operation or model/settings migration is introduced.
Packaged resources remain package-owned; separately downloaded shared models are not caches
and are outside this installer's ownership.

## Repeatable metadata tests

On Windows with WiX 5.0.2 and its matching UI/Util extensions:

```powershell
./packaging/windows/test-install-policy.ps1 -OutputDirectory ./target/install-policy-tests
```

The script builds x64, x86 and arm64 packages with deliberately non-executable payloads. It
uses the actual package source and compiled database. A safe Windows Installer session checks
24 scope/registration/maintenance cases, remembered and explicit custom folders, sequencing,
upgrade identity and the original registration inventory recorded in `install-policy.json`.
The release packaging script runs the compiled checker before signing. Packaging CI runs the
small fixture builds without compiling the Rust application.

Never install or distribute the `NOT-INSTALLABLE.msi` fixtures as an app. Passing these checks
does not prove real installation, elevation, legacy-version upgrades, registry writes, optional
shortcut/menu behaviour, cancellation, rollback or uninstall. Those still require the real
payload in an authorised clean Windows test environment. Cross-architecture metadata is not
execution evidence on those architectures. Existing open installer/association PRs must be
reconciled before publication. Nothing here changes macOS/Linux native packaging conventions.
