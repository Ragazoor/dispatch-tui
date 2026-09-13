This is a CVE remediation task. It is a targeted dependency fix, not a feature.
Do not design it spec-first, and do not write a failing test first — neither
matches the shape of this work.

1. Read the advisory. The description carries the summary; the task's URL points
   at the alert. Identify the vulnerable package, the affected range, and the
   exact versions the advisory lists as fixed.
2. Find where this repo pins that package — a manifest, a lockfile, or a
   transitive dependency of something it pins.
3. Apply the smallest change that lands on a fixed version. A higher version
   number is not automatically patched: confirm the version you land on is in
   the advisory's fixed set. Fixes are backported to maintenance branches, so a
   newer minor released earlier can still be vulnerable.
4. If no fixed version exists, or reaching one needs source changes beyond the
   pin, stop and tell the user what you found. Do not invent a workaround.
5. Run the repo's verify command and confirm it passes.
6. Add a test only where the fix changed our own code rather than a pinned
   version. The rule above is against test-first design of a version bump, not
   against testing a patch you wrote yourself.
