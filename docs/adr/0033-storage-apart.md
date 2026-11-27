# ADR-0033: vØrk storage in a repository of its own, with an app of its own

- **Status:** accepted
- **Date:** 2026-11-27
- **Irreversible:** no

## Context
vØrk storage came into this repository on 2026-11-18 (ADR-0015, ADR-0022), was released with vØrk under vØrk's version from 0.4.0, and showed its state and settings as pages in the node's dashboard (ADR-0032).

Releasing both from one repository put two Android apps in each release. Updaters that follow a repository's releases (Obtainium) take one APK per repository, so updating the storage app that way didn't work. Storage also changes at its own pace: a release of the network waited for storage's packages, and the other way round.

## Decision
1. **Storage moves to its own repository**, [vork-storage](https://github.com/vooteleaer/vork-storage), with its history since 2026-11-18: its crates, its apps, its packaging (MSI, .deb, Docker) and its documents. It has its own releases. vØrk 0.6.0 is the first release without it.
2. **vØrk keeps the local API** (`crates/vork-api`), the interface every application uses. Storage takes it from this repository, pinned to a release tag.
3. **Storage gets an app of its own** instead of pages in the dashboard: a window of its own on the desktop, its own app on Android, and its own app with its own web page on a NAS (TrueNAS) and in Home Assistant.
4. **The node stays application-agnostic.** ADR-0032's application pages stay: any application may still use them. The `vork` command still passes storage's commands to `vork-storage` when it's installed.

## Consequences
- Each repository's releases carry one app per platform, so Obtainium (or any updater following releases) can update each.
- vØrk and storage can be released apart. Their compatibility is the local API's: storage names the vØrk release whose API it was built against.
- Storage's Android version codes continue from the numbers its installs already have (the commit count of this repository, up to 210), so updates install over them.
