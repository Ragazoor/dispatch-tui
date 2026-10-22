1. Find where this repo pins that package — a manifest, a lockfile, or a
   transitive dependency of something it pins.
2. Apply the smallest change that lands on a version in the fixed set.
3. If no fixed version exists, or reaching one needs source changes beyond the
   pin, stop and tell the user what you found. Do not invent a workaround.
4. Run the repo's verify command and confirm it passes.
5. Add a test only where the fix changed our own code rather than a pinned
   version. The rule above is against test-first design of a version bump, not
   against testing a patch you wrote yourself.
