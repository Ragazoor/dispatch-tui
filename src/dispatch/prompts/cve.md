This is a CVE remediation task. It is a targeted dependency fix, not a feature.
Do not design it spec-first, and do not write a failing test first — neither
matches the shape of this work.

Start from the advisory. The description carries the summary; the task's URL
points at the alert. Identify the vulnerable package, the affected range, and
the exact versions the advisory lists as fixed. Only a version in that fixed set
counts: a higher version number is not automatically patched, because fixes are
backported to maintenance branches and a newer minor released earlier can still
be vulnerable.

{{STEPS}}
