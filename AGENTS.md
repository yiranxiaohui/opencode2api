# Repository Guidelines

## Project Structure & Module Organization

The Rust backend lives in `src/`. `main.rs` starts Axum, `state.rs` owns shared state, `db.rs` contains SQLite access, and `models.rs` defines API types. HTTP handlers are under `src/routes/`; add schema changes as ordered migrations in `src/migration/mod.rs`.

The React frontend lives in `frontend/src/`. Put reusable UI in `components/`, HTTP wrappers and types in `api/`, query hooks in `hooks/`, and global styling in `styles.css`. Do not commit generated `frontend/dist/`. Rust unit tests are colocated in `#[cfg(test)]` modules.

## Build, Test, and Development Commands

- `cargo run` — run the backend at its configured local address.
- `cargo test --locked` — execute backend and migration tests using `Cargo.lock`.
- `cargo fmt --all -- --check` — verify Rust formatting.
- `cargo clippy --locked --all-targets` — run Rust static analysis.
- `cd frontend && bun install --frozen-lockfile` — install the pinned frontend dependencies.
- `cd frontend && bun run dev` — start the Vite development server.
- `cd frontend && bun run lint` — run Oxlint.
- `cd frontend && bun run build` — type-check and create the production frontend bundle.
Run the same formatting, lint, test, and build checks used by `.github/workflows/ci.yml` before opening a pull request.

## Coding Style & Naming Conventions

Use standard `rustfmt` output and Rust naming conventions: `snake_case` functions/modules, `PascalCase` types, and `SCREAMING_SNAKE_CASE` constants. Keep route handlers focused; place shared persistence logic in `db.rs` and shared response types in `models.rs`.

Frontend code uses TypeScript, functional React components, two-space indentation, and `PascalCase.tsx` component filenames. Use `camelCase` for functions and state. Reuse API types from `frontend/src/api/types.ts` instead of duplicating response shapes.

## Testing Guidelines

Add focused unit tests for protocol conversion, usage parsing, encryption, and migrations. Name tests after observable behavior, for example `extracts_cache_usage_from_all_supported_protocols`. Schema changes must test both fresh database creation and upgrade behavior. No fixed coverage threshold exists, but changed logic should have regression coverage.

## Commit & Pull Request Guidelines

Recent commits use short, imperative subjects such as `Add cache token usage to request logs`. Keep each commit scoped to one coherent change. Pull requests should explain behavior and migration impact, list commands run, link relevant issues, and include screenshots for visible UI changes. Never commit API keys, SQLite data, generated bundles, or proxy credentials.

## Required Development Workflow

For every code change, first update local `master` from `origin/master`, then create a dedicated feature or fix branch in its own Git worktree. All implementation, formatting, testing, and commits must happen inside that isolated worktree; do not edit code in the primary worktree or directly on `master` unless the user explicitly requests an exception.

After verification, create a focused commit, merge the completed branch into local `master`, and push `master` to `origin`. Once the merge and push succeed, remove the task worktree, delete the merged local branch, and delete its remote branch if one exists. At handoff, report the worktree path, working branch, commit hash, checks run, merge result, push status, worktree removal, and branch cleanup result.
