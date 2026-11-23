# ADR-0032: Applications add pages to the node's dashboard

- **Status:** accepted (built 2026-11-23)
- **Date:** 2026-11-23
- **Irreversible:** no

## Context
Applications on a machine use its node through the local API (ADR-0020) instead of running their own. Such an application needs somewhere to show its state and settings. A window of its own on every platform means one interface per platform (Win32, GTK, Haiku, Android), each a program to keep up. The node already has one interface everywhere: the dashboard, the same page on the web, in the vØrk window and in the Android app.

An application that isn't installed shouldn't show up anywhere: the dashboard can't know of any particular one.

## Decision
1. **An application says its pages** over the local API (`PAGES id title {path title}…`); the dashboard adds them to its menu under the application's title. The node asks the application for a page's content (`PAGE`), the application answers (`RESPOND`). The node knows nothing about any one application.
2. **Remembered:** the node keeps what each application said in its state directory. After the application disconnects (or the node restarts) its pages stay in the menu, shown as not running, until it says them again; it removes them by saying none, and the owner can remove those of one that isn't running.
3. **Sandboxed:** a page is shown in a frame of its own origin (`sandbox="allow-scripts"`). It can't reach the dashboard, its key or the node; it talks only to its own application, through the dashboard (`vork.call`), and can show another of its pages (`vork.go`). It's given the dashboard's colors, so it looks like a part of it.
4. **No approval:** any program that may use the local API may add pages, as any may use the network. Each page says which application it comes from.
5. **Through the dashboard's own channel:** pages and calls go as a dashboard command (`{"cmd":"app"}`), so they work wherever the dashboard does, the window's and the Android app's bridges included, and only for the dashboard's owner.

## Consequences
- One interface for an application on every platform the dashboard runs on; native code stays only where the platform needs it (file systems, background services).
- A local program can add a page that looks like a part of the dashboard. It can't do anything the dashboard can (sandbox), and its menu entry and header name it; the owner can remove it once it isn't running.
- An application's page is as responsive as the application: the dashboard waits 15 s for an answer, then says it didn't come.
- A page lives in a frame: it keeps its state between the dashboard's refreshes, and its height follows its content.
