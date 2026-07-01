---
type: Reference
title: Bastion Setup & Connection Guide
description: Step-by-step guide to provisioning the database, running migrations, and configuring bastion to connect to the orchestrator ecosystem.
doc_id: bastion-setup
layer: [console]
project: bastion
status: active
keywords: [setup, configuration, postgres, database, orchestrator, connect, migrations]
related: [config, monitor]
---

# Bastion Setup & Connection Guide

The `bastion` CLI is the control panel for the whole ecosystem. Features like `bastion monitor`, `bastion inspect`, and `bastion run` rely on connecting to the `orchestrator`'s PostgreSQL database and API server.

Because `bastion` acts as a pure read/write client over the database and does **not** manage schema migrations itself, you must provision the database from the `orchestrator` repository first.

Here is the end-to-end setup guide to get `bastion` connected and working.

## Step 1: Provision the Database (via Orchestrator)

The `orchestrator` repo contains an idempotent setup script that installs Postgres and Redis via Homebrew, creates the database/user, enables `pgvector`, and runs the Alembic schema migrations.

1. Navigate to the orchestrator:
   ```bash
   cd ../orchestrator
   ```
2. Run the local dev setup script:
   ```bash
   ./scripts/dev-setup.sh
   ```
   *This script does the following:*
   - Installs `postgresql@17`, `redis`, and `pgvector` via Homebrew.
   - Creates the `orchestration_dev` database and user `orchestration` (password: `orchestration`).
   - Runs `alembic upgrade head` to apply all migrations so the schema is ready for `bastion` to query.
   - Generates the `.env` file for the orchestrator.

## Step 2: Configure Bastion

`bastion` needs to know how to connect to the newly created database and the orchestrator's API. The most secure and flexible way to provide the connection string is via the `DATABASE_URL` environment variable.

1. Export the variables in your shell (or add them to your `~/.bashrc` / `~/.zshrc`):
   ```bash
   export DATABASE_URL="postgres://orchestration:orchestration@localhost:5432/orchestration_dev"
   export BASTION_API_URL="http://localhost:8080"
   ```

2. (Optional) Set up workspace roots in `~/.config/bastion/config.toml`:
   ```toml
   # Only needed if you use 'bastion brain' queries
   default_workspace = "brain"
   
   [workspaces]
   brain = "/Users/<your-username>/Dev/agentic-portfolio"
   ```

## Step 3: Run the Orchestrator Engine

`bastion monitor` and `bastion inspect` read directly from the database and will work immediately once the migrations are applied. However, to see *live* workflows or use `bastion run`, the orchestrator needs to be running.

From the `orchestrator` directory, spin up the stack:
```bash
./scripts/dev.sh
```
*(This starts the FastAPI server and Celery workers inside a tmux session).*

## Step 4: Validate the Connection

Now that the database exists, migrations are applied, and `bastion` is configured, you can test it from anywhere:

```bash
# Verify the database connection and view static workflows
bastion inspect

# Watch live workflow execution (requires the orchestrator to be running tasks)
bastion monitor

# Trigger a workflow
bastion run <workflow_name>
```

---

## Troubleshooting

- **Error: `relation "task_context" does not exist`**
  This means Bastion connected to the database, but the Alembic migrations haven't run. Go back to `orchestrator` and ensure `./scripts/dev-setup.sh` completes successfully, or run `uv run python -m alembic upgrade head` manually in `orchestrator/app`.
- **Error: `connection to server on socket "/tmp/.s.PGSQL.5432" failed`**
  Postgres isn't running. Start it via `brew services start postgresql@17`.
- **Config file ignored?**
  Ensure your config file is at `~/.config/bastion/config.toml` (or `$XDG_CONFIG_HOME/bastion/config.toml`). A malformed TOML file will cause a hard exit, but a missing file will silently fall back to environment variables.
