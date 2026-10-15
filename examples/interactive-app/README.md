# Interactive Flowboard

This example uses the same `agena-ui` reply protocol as assistant output. It has a responsive inline UI and a real Python standard-library + SQLite task API. There are no packages to install.

From the Agena repository workspace, run `python3 examples/interactive-app/reply.py` to obtain the complete reply fence. An assistant can include that fence in its answer. Choose **Run app** on the resulting card. Agena allocates a free loopback port, starts `server.py`, waits for `/health`, then boots the UI. Add, complete, search, filter and delete tasks. **Open in preview** opens the same app in a workspace tab; **Service logs** shows server output; **Stop** stops its managed process group.

Tasks persist in `flowboard.sqlite` beside `server.py` across stop/start. Search and filter state persist separately in Agena's server database, scoped to the conversation and app id. The HTML has no direct network or host access; all API requests go through `window.agena.fetch('api', path, init)`.

The backend can also be exercised independently: `python3 examples/interactive-app/server.py --port 8471 --database /tmp/flowboard.sqlite`. `GET /tasks`, `POST /tasks` (`{"title":"Task"}`), `PATCH /tasks/1` (`{"done":true}`), and `DELETE /tasks/1` perform real database operations. Bindings are loopback only.
