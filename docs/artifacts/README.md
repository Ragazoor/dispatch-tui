# Board graph artifact

`board-graph.html` is the template for the live board graph artifact. The
`/*DATA*/…/*END*/` block holds the data. A dispatched agent refreshes the
artifact by building `{generated, epics:[{id,name,parent,t:{b,r,v,d}}],
packages:[{id,title,items:[epic ids]}]}` from the MCP tools (`list_epics`,
`list_tasks`, `list_work_packages`), replacing that block, and republishing to
the artifact URL stored in the phoenix task's description.
