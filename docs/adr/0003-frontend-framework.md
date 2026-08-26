# ADR 0003: Preact and Vite for the Static Frontend

- **Status:** Accepted provisionally
- **Date:** 2026-08-25

## Context

Helix needs a polished customizable dashboard with routing, forms, live data, drag/resize interactions, permissions, themes, accessibility, and eventually game-specific and extension-provided interfaces. It also promises a small initial payload, immediate shell rendering, and no Node.js runtime dependency on the managed server.

A no-framework frontend minimizes framework bytes but shifts state, component, lifecycle, accessibility, and testing conventions into project-specific code. A large framework or component suite would make initial delivery easier at the cost of permanent download, parsing, memory, upgrade, and design-system weight. The application does not need server-side rendering: `helixd` already serves a local API and static assets.

The initial choice must be reversible before the UI becomes broad. Popularity is not sufficient evidence; bundle and interaction measurements are required.

## Decision drivers

- Small initial JavaScript and CSS payload.
- Static production assets with no Node.js runtime.
- Strong TypeScript and component composition for a long-lived interface.
- Route-level lazy loading and tree shaking.
- Testability of loading, error, reconnect, and permission states.
- Accessible semantic output without a heavyweight component framework.
- A shallow integration boundary between frontend and versioned API.
- Low migration cost if early measurements disprove the choice.

## Options considered

### Vanilla TypeScript and Web Components

This offers the smallest dependency floor and native platform primitives. The expected dashboard complexity would require Helix to define more rendering, state, composition, testing, and compatibility conventions itself. That may save framework bytes while increasing application code and maintenance risk. Retained as a viable fallback for isolated widgets, not selected for the application shell.

### Svelte

Svelte provides a compiler-oriented component model and can produce small output. It introduces its own compilation and reactivity semantics, and output depends on application shape. It remains a credible alternative that should be included in an equivalent-shell comparison if Preact misses the budget.

### Solid

Solid offers fine-grained reactivity and compact runtime behavior. Its ecosystem and programming model would be another reasonable fit. It remains an alternative for the validation comparison.

### React

React has a large ecosystem, but the full runtime and common supporting stack are unnecessary for the local dashboard foundation when a compatible smaller component model is available. Rejected for the base shell unless a future required capability demonstrates a measured net advantage.

### Preact with TypeScript and Vite

Preact provides a compact component runtime and familiar typed JSX model. Vite produces static, content-hashable assets and supports dynamic imports. The current authenticated dashboard remains small and avoids an application framework, server renderer, global state library, CSS runtime, and component suite. Chosen provisionally.

## Decision

The foundation frontend uses:

- Preact for components and rendering;
- TypeScript with strict checking;
- Vite as a development and production build tool;
- native CSS with design tokens and deliberate component styles;
- Vitest for unit/component behavior where appropriate;
- browser-level end-to-end tests for critical flows once those flows exist.

Node.js is used only to build and test. Production packages contain compiled HTML, CSS, JavaScript, and other static assets served by `helixd`. Node and Vite are not installed on the target host.

The frontend is a client of the versioned HTTP API. It does not import Rust implementation details, read SQLite, form trusted filesystem paths, or become authoritative for roles, service state, jobs, backups, or configuration. Generated or shared API schemas may inform types, but runtime trust boundaries still validate unsafe input.

No server-side rendering or hydration layer is introduced in the foundation. The local shell should render immediately from static assets, then show explicit loading and reconnect states while fetching authoritative data.

## Payload constraints

The initial route contains only:

- application shell and navigation;
- theme and accessibility primitives;
- authentication, setup, and session-expired surfaces;
- small real host summary components;
- error, loading, disconnected, and empty states.

These features are separate dynamic imports and do not enter the initial chunk:

- chart engine and historical metric views;
- terminal or console;
- file editor and syntax support;
- drag-and-resize dashboard editor when not active;
- game-specific interfaces;
- Strand UI;
- advanced search and command palette.

The base frontend does not add a general component framework, icon pack, web font, runtime CSS-in-JS system, rich editor, or global state library without a measured ADR. Icons should use a small reviewed local set. Immutable assets use content hashes, Brotli with gzip fallback, long-lived caching, and ETags where the serving implementation supports them; the HTML shell is revalidated.

## Provisional validation gate

This ADR accepts the implementation direction but not a “lightweight” claim. Before Phase 1 expands the screen count:

1. record raw, gzip, and Brotli sizes for the entry HTML, CSS, initial JavaScript, and source-map-excluded production payload;
2. record build conditions and exact dependency versions from the lockfile;
3. measure cold and warm shell render, parse/execute cost, and idle browser activity on a documented low-end client;
4. confirm route-level chunks exclude charts, terminal, editor, games, and Strands;
5. create equivalent representative shells with the most credible alternative if the budget is missed or framework overhead dominates;
6. set numeric warning and failure budgets in `docs/PERFORMANCE.md` and CI.

Source maps are a release-packaging decision: they may be retained as separate debugging artifacts, but they must not accidentally expose machine paths, secrets, or unpublished source through the production server.

If Preact fails the documented budget or makes accessibility and interaction behavior materially harder, replace it before broad feature work. Existing screens do not outweigh measured product constraints at this stage.

## State and data flow

Server resources are cached only as projections with revisions. A successful optimistic UI animation never commits server state. Mutations reconcile against the returned resource or job and handle conflict, denial, partial availability, and disconnect explicitly.

Local storage is limited to non-sensitive user preferences that are safe before authentication. Sessions use protected cookie mechanisms where implemented. Bearer tokens, passwords, recovery keys, MFA secrets, RCON credentials, raw configurations containing secrets, and backup keys are never placed in local storage.

Live metrics can update bounded in-memory view state. Historical data is requested by range. Disconnected tabs reduce or stop live work. Reconnect performs an authoritative refetch rather than assuming every transient event was received.

## Accessibility and theming

Components use semantic HTML before ARIA, remain keyboard operable, expose visible focus, and work with browser zoom and reflow. Motion respects reduced-motion preferences. Color is never the only status signal. OLED and dark themes still meet contrast and disabled-state clarity requirements.

Theming uses versioned design tokens. A theme or Strand cannot inject unrestricted script or force the core Content Security Policy to allow unsafe execution. User customizations are validated and bounded.

## CIA consequences

### Confidentiality

Static assets contain no secrets or environment-specific private data. Sensitive API responses use appropriate cache controls and are not persisted in browser storage. A compromised same-origin script can act as the user, so dependency review, CSP, output escaping, and minimal third-party code remain required.

### Integrity

The server enforces all permissions and revisions. The build uses pinned dependencies and should produce reviewable hashed assets. Frontend validation improves feedback but is never trusted by the API.

### Availability

The compiled shell remains independent of Node.js and optional feature chunks. Lazy loading limits initial failure and resource cost, but a frontend exception can still make the interface unusable; error boundaries, retry states, static-asset/API compatibility tests, and rollback-compatible packaging are required. Managed game workloads remain unaffected.

## Consequences

Positive:

- small starting dependency surface and static deployment;
- component model suitable for the planned dashboard;
- route-level code splitting through ordinary dynamic imports;
- familiar TypeScript tooling and test support;
- no production JavaScript runtime service.

Costs:

- Preact compatibility assumptions require testing for any React-oriented third-party library;
- a client application still consumes memory after load and needs explicit idle-work control;
- Vite and npm dependencies add a build-time supply chain;
- custom accessible components require careful design and testing;
- no SSR means the shell must design network-loading states well.

The project accepts those costs while keeping the dependency set deliberately narrow.

## Validation

- lock dependency versions and audit the final production graph;
- fail CI on type, lint, unit, and production-build errors;
- record chunk composition and compressed sizes;
- test keyboard navigation, focus, contrast, zoom/reflow, reduced motion, and screen-reader-critical flows;
- test stale assets against an upgraded API and define compatibility/refresh behavior;
- verify CSP, escaping, cookie/CSRF behavior, cache headers, and source-map packaging;
- measure idle timers, CPU, memory, event frequency, and hidden-tab behavior;
- run core flows at mobile and desktop widths.

## Revisit triggers

Revisit the choice if the initial budget is missed, required libraries pull most of React into the bundle, Preact compatibility defects recur, accessibility work is blocked by the component model, or an alternative representative shell is materially smaller or simpler under the same requirements. Adding a large framework layer or state system requires a new ADR rather than silently eroding this decision.
