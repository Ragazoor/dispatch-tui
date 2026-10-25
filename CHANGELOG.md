# Changelog

## [0.4.0](https://github.com/Ragazoor/dispatch-tui/compare/v0.3.0...v0.4.0) (2026-10-23)


### Features

* **agent-tree:** two views, full-width lower pane, editor key ([a7ebe53](https://github.com/Ragazoor/dispatch-tui/commit/a7ebe5311577e91942502ed3d3486240ff3ccd42))
* **feeds:** link each CVE alert to the open PR that fixes it ([32500da](https://github.com/Ragazoor/dispatch-tui/commit/32500da04e91e0f3a73a98893f78e0664228c1ba))
* **feeds:** say how fetch-cve.sh matched a fix PR ([6266e21](https://github.com/Ragazoor/dispatch-tui/commit/6266e21cb2387c2fd35e41d604c1f572d633ea2b))
* **hooks:** fail the pre-push hook early when rustc ignores the pin ([8417103](https://github.com/Ragazoor/dispatch-tui/commit/8417103623d4099e52b27bd1d5c9f8b4a22d63b7))
* **learnings:** one verdict per task and learning, latest wins ([e9e1106](https://github.com/Ragazoor/dispatch-tui/commit/e9e1106696922f3c4c4dacad34433515e0106a57))
* **mcp:** move agent-name conventions into MCP server instructions ([5a047ef](https://github.com/Ragazoor/dispatch-tui/commit/5a047ef8e2b9fb926c995f3d244348030fae6f19))
* schedule tasks with a five-field UTC cron ([054df32](https://github.com/Ragazoor/dispatch-tui/commit/054df327852dd63f523e02c105cfdd01139d184f))
* signal a new task in an epic until the user enters it ([dfdce54](https://github.com/Ragazoor/dispatch-tui/commit/dfdce54fe5f62e27d54637abe03ab191c70ef939))
* **tui:** apply the flat lens in a work package view ([b6bccfd](https://github.com/Ragazoor/dispatch-tui/commit/b6bccfda2e5da179f010d40a60b7262f143e8503))
* **tui:** lead task cards with the current agent's name ([bfb36c4](https://github.com/Ragazoor/dispatch-tui/commit/bfb36c45475650818b8b4b9480e3fe52c0f7a82c))
* **tui:** link a task created in a package view to that package ([3651374](https://github.com/Ragazoor/dispatch-tui/commit/3651374d4b90f1ec23c22dd148331cda379fb06c))
* **tui:** name the agent in the crash status message ([451f4b9](https://github.com/Ragazoor/dispatch-tui/commit/451f4b94408ac0fc2efc2b8031077f7043a31e0a))
* **tui:** reorder work package cards with J/K via a sort order ([4c8a13b](https://github.com/Ragazoor/dispatch-tui/commit/4c8a13ba5e642e262d8095f684ffa5ac754c3c34))
* **work-packages:** create and update are one reducer transaction each ([997d957](https://github.com/Ragazoor/dispatch-tui/commit/997d957779673c962f526f92492ae79a58ce857b))


### Bug Fixes

* adopt only a real store, refuse moves to missing epics, close test gaps ([bf42341](https://github.com/Ragazoor/dispatch-tui/commit/bf42341e20ac79e7845ef2886061ba45800b3983))
* **cli:** prune-repo-paths only judges repo paths this host saved ([1d8c6dc](https://github.com/Ragazoor/dispatch-tui/commit/1d8c6dc617681c14482bd5003e883cc2d6d94195))
* **deps:** bump async-trait to 0.1.92 for clippy 1.99 ([e4d388f](https://github.com/Ragazoor/dispatch-tui/commit/e4d388f3a3274b225658baa0c4ff9f42ad23d563))
* **feeds:** only link CVE fix PRs from known dependency bots ([ee9504f](https://github.com/Ragazoor/dispatch-tui/commit/ee9504f6c9838670bf520d7a2e9712bdd4f499c0))
* harden push classification, symlink writes and worktree git handling ([d84083f](https://github.com/Ragazoor/dispatch-tui/commit/d84083f6bceac5235474a39deae4e504cf3d86d9))
* **mcp:** harden the origin guard, require JSON on the hook route ([a854286](https://github.com/Ragazoor/dispatch-tui/commit/a85428656f5e2231e1595c525907f60ae16ebc5e))
* review cleanups from the 2026-10-22 follow-up ([26a3b89](https://github.com/Ragazoor/dispatch-tui/commit/26a3b89f753f5a897a46625e0b9e839cb6d6a425))
* **skills:** keep Low findings and decision items in decompose-review ([e604909](https://github.com/Ragazoor/dispatch-tui/commit/e6049095b69fcbf5313ff20ae2a89795f52e4bce))
* **split:** rejoin a pinned pane into the window it was pinned from ([1f96c69](https://github.com/Ragazoor/dispatch-tui/commit/1f96c697139e136ad5d1dfdeff67d5a9f1de7969))
* **split:** settle a swap on what tmux did when a step after swap-pane fails ([6451392](https://github.com/Ragazoor/dispatch-tui/commit/64513924741a59299308c68197194b35e587b1e2))
* stop tmux errors reading as absent windows; tidy feed and editor cleanup ([93e8cd9](https://github.com/Ragazoor/dispatch-tui/commit/93e8cd9cb0fbcb7c3de712a5fb4b014db8b837e7))
* **tui:** correct task-detail scroll bound and char-safe hint labels ([2d73aa5](https://github.com/Ragazoor/dispatch-tui/commit/2d73aa52279ec9a8a7b73f275114078af1fbb9e0))

## [0.3.0](https://github.com/Ragazoor/dispatch-tui/compare/v0.2.0...v0.3.0) (2026-10-16)


### Features

* **agent-tree:** derive the companion pane's tree from git ([e8cbc25](https://github.com/Ragazoor/dispatch-tui/commit/e8cbc255e813b64a18bcf936bb3108b2ef4ef621))
* **cli:** find a named store beside the DB; panes read through the board ([9b62019](https://github.com/Ragazoor/dispatch-tui/commit/9b620194c9837480a85e7b37c725b08f1ed251da))
* **db:** attach the memory store in Database::open_in_memory ([2103c99](https://github.com/Ragazoor/dispatch-tui/commit/2103c99337dedb8b26aa3930d9afe8c4fc3f35a0))
* **epics,spacetime-seed:** server-side DeleteEpic guard + restore refusal for archived rows (task [#4971](https://github.com/Ragazoor/dispatch-tui/issues/4971)) ([eb39c11](https://github.com/Ragazoor/dispatch-tui/commit/eb39c11e3b3c7a69710a4c5b958ffda1bde00017))
* **epics:** name the blocking task when an epic delete is refused ([e00d4f1](https://github.com/Ragazoor/dispatch-tui/commit/e00d4f17a18bc1197de1cffe6075b56a9ac6ad7e))
* **feeds:** default log-warning cards to the rebase wrap-up path ([6833aa5](https://github.com/Ragazoor/dispatch-tui/commit/6833aa5005e4a24349a91a4b34085478810ea214))
* **feeds:** RetiredFeedItem table, ingest suppression, and delete-time retirement (task [#4971](https://github.com/Ragazoor/dispatch-tui/issues/4971)) ([7969400](https://github.com/Ragazoor/dispatch-tui/commit/79694001a1bf70791b13d3b1ad51b4883e071366))
* gate MockProcessRunner behind test-support cargo feature ([d9440e7](https://github.com/Ragazoor/dispatch-tui/commit/d9440e7691e9b87d2f9ab4ab77be17e37d62dba3))
* **host:** pin a task's worktree to the machine that holds it ([bf034d4](https://github.com/Ragazoor/dispatch-tui/commit/bf034d4f1a70364339ff30ee8e42313934d751f6))
* **keybindings:** one table for every key binding, list_keybindings MCP tool ([40ae956](https://github.com/Ragazoor/dispatch-tui/commit/40ae9561cc488020df08ef89e6813db82f89a1e6))
* **mcp:** get_epic surfaces parent epic's id and title (task [#8261](https://github.com/Ragazoor/dispatch-tui/issues/8261)) ([f7ab8b0](https://github.com/Ragazoor/dispatch-tui/commit/f7ab8b0006f3c76c791be7250a896c638d58e648))
* **models:** newtype TmuxWindow for tmux window names ([f180523](https://github.com/Ragazoor/dispatch-tui/commit/f18052329f6e6c74f98877425fe88dd13481b1f2))
* **prompts:** dependabot runbook gates merge on kognic-github-app verdict ([0c4ebb6](https://github.com/Ragazoor/dispatch-tui/commit/0c4ebb63bce9b5b1e1f2322786946a2ad5727150))
* **spacetime:** atomic BatchDelete reducer, document refuse-too-late as accepted limitation (task [#4971](https://github.com/Ragazoor/dispatch-tui/issues/4971)) ([581e889](https://github.com/Ragazoor/dispatch-tui/commit/581e88963c0c07b64e0977ed7f1c94cd313edd50))
* **spacetime:** implement MemoryReducerCaller for tasks_and_epics/repo_config/subscriptions (task [#4975](https://github.com/Ragazoor/dispatch-tui/issues/4975)) ([131dd6c](https://github.com/Ragazoor/dispatch-tui/commit/131dd6c76f2cc405d005322080d5daeea68e9c02))
* **spacetime:** MemoryReducerCaller covers learnings, retrievals + verdicts (task [#5003](https://github.com/Ragazoor/dispatch-tui/issues/5003)) ([cc1b0c5](https://github.com/Ragazoor/dispatch-tui/commit/cc1b0c56f1d8a5e0b30d83a77f6921e2a1ee7505))
* **spacetime:** MemoryReducerCaller covers settings (task [#5002](https://github.com/Ragazoor/dispatch-tui/issues/5002)) ([4db524e](https://github.com/Ragazoor/dispatch-tui/commit/4db524e90e5ea10844738a2631687b1f954bb40e))
* **spacetime:** MemoryReducerCaller covers usage + agent_state (task [#5004](https://github.com/Ragazoor/dispatch-tui/issues/5004)) ([259ab4b](https://github.com/Ragazoor/dispatch-tui/commit/259ab4b194a457f6b68e6063040e3cbe30657c28))
* **spacetime:** remove unused create_retired_feed_item reducer, fix task-delete cleanup gap (task [#4971](https://github.com/Ragazoor/dispatch-tui/issues/4971)) ([27ce6e9](https://github.com/Ragazoor/dispatch-tui/commit/27ce6e9e7dc3dfd67a5e3cc4694bc322dfc62b93))
* **spacetime:** server-side DeleteTask guard, RefuseImplausibleCeiling spec (task [#4971](https://github.com/Ragazoor/dispatch-tui/issues/4971)) ([7d4e779](https://github.com/Ragazoor/dispatch-tui/commit/7d4e7792378d3d2f9bfb2c0b205c3478e877b3b5))
* **spacetime:** shared-store RetiredFeedItem support, finish archive removal (Area 4, task [#4971](https://github.com/Ragazoor/dispatch-tui/issues/4971)) ([9edbdb5](https://github.com/Ragazoor/dispatch-tui/commit/9edbdb5e13e63f8d44b37c2f4d98d2051dec77ee))
* **startup:** dispatch tui runs and updates its own local store ([41f876c](https://github.com/Ragazoor/dispatch-tui/commit/41f876cdbda332142e5b722c57e7070d0e628f3f))
* **store:** import the old store's rows into the managed store ([8b62240](https://github.com/Ragazoor/dispatch-tui/commit/8b6224013ddd4c29bf8cf053c69b17900d276805))
* **tasks:** remove TaskStatus::Archived and every reader (Area 3, task [#4971](https://github.com/Ragazoor/dispatch-tui/issues/4971)) ([9953aea](https://github.com/Ragazoor/dispatch-tui/commit/9953aea2c5af310e9b1d4c29108b57762b0635e0))
* **tui:** jump straight to nested epics ([53feee3](https://github.com/Ragazoor/dispatch-tui/commit/53feee3b65a560626e3cd233441c29f223226f25))
* **tui:** make the phoenix creation step a [p]/[Enter] picker ([9fb823d](https://github.com/Ragazoor/dispatch-tui/commit/9fb823dffd9810b92b0d57667d33a51af0052d41))


### Bug Fixes

* **agent-tree:** resolve the diff baseline against both base refs ([1ad478c](https://github.com/Ragazoor/dispatch-tui/commit/1ad478cbfc65ea0c37a90ec87c36ca067a6ad5be))
* **agent-tree:** wait for the task row instead of closing the pane ([519d1ff](https://github.com/Ragazoor/dispatch-tui/commit/519d1ffcbff9ed34d154709d6326e140094900c5))
* align allium specs with implementation, close three real bugs ([5a21f18](https://github.com/Ragazoor/dispatch-tui/commit/5a21f18fafa49b6177af236781b210879f3ed448))
* **cli:** log the cause when the agent-tree/agent-diff git query fails ([e9b51d1](https://github.com/Ragazoor/dispatch-tui/commit/e9b51d12f6c6dc0deb13a68c563999721d72ccea))
* **delete:** pre-check the store's true status before any teardown or purge ([c818857](https://github.com/Ragazoor/dispatch-tui/commit/c818857a800f781d9218c330a7b29a7b8358e5f8))
* **docs:** keep a test name on one line so the doc-symbols check resolves it ([b04c723](https://github.com/Ragazoor/dispatch-tui/commit/b04c723322a08310d7caeb16a045011104e52460))
* **feeds:** weed fixes for RetiredFeedItem migration + renumber v103/v104 after rebase ([495b96a](https://github.com/Ragazoor/dispatch-tui/commit/495b96a6f076bbbcc5aab186ce27911bc3e16d9e))
* make plain cargo build pass by gating the memory store ([950a7ba](https://github.com/Ragazoor/dispatch-tui/commit/950a7bacc9526c27809a1c3ece44ddb784546ad9))
* **migrations:** v106 phase 4 deletes an empty subtree's non-archived children too ([2b954b8](https://github.com/Ragazoor/dispatch-tui/commit/2b954b8330c1b6a2d77ecd6f50cf389ef07932e1))
* pin the budget snapshot location to the default data directory ([fa1e3a2](https://github.com/Ragazoor/dispatch-tui/commit/fa1e3a22e68063a8b655ee6b7c9ab408cd0c770a))
* **service:** update_task reports not-found for a missing task on the shared store ([a456d25](https://github.com/Ragazoor/dispatch-tui/commit/a456d2501560e101f1c161feff434f2f2eecdb42))
* **spacetime:** integrate RetiredFeedItem with the now-mandatory store, fix burn_id_sequence gap (task [#4971](https://github.com/Ragazoor/dispatch-tui/issues/4971)) ([8d9c36d](https://github.com/Ragazoor/dispatch-tui/commit/8d9c36de6fa3ba79b09dbd403395c5fd7a4fc86e))
* **spacetime:** module refuses task and epic writes with an unknown status ([126194f](https://github.com/Ragazoor/dispatch-tui/commit/126194f4cf40a92c718d7fb89b81bded97e45792))
* **sync:** drop rows with a removed status quietly ([72c2573](https://github.com/Ragazoor/dispatch-tui/commit/72c2573487627296adee0b77ab4702bcb531dcaa))
* **sync:** reconcile MemoryReducerCaller with [#4971](https://github.com/Ragazoor/dispatch-tui/issues/4971)'s delete-guard changes after rebase ([6b34df3](https://github.com/Ragazoor/dispatch-tui/commit/6b34df36c572e57d64b2316ca02e602ff64643f4))
* **sync:** unfollowing a feed epic releases the unfollower's poll claims ([8ea0e72](https://github.com/Ragazoor/dispatch-tui/commit/8ea0e721498bbfd65bdc2f129a431139556d1dbc))
* **tui:** clip settled lines in the New Task form to one row ([f701bba](https://github.com/Ragazoor/dispatch-tui/commit/f701bba2c7c30e30556f0abf83dc6cb3f86b7557))
* **tui:** let columns fill the board when the input panel is idle ([9ea5ac2](https://github.com/Ragazoor/dispatch-tui/commit/9ea5ac221c816adc9badc0491af165880305416e))
* **tui:** render the phoenix step inside the creation-form panel ([be1f2f5](https://github.com/Ragazoor/dispatch-tui/commit/be1f2f534351c7037f2b3cce5a1e729f4d034290))
