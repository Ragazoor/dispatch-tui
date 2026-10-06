This is a minor bump, so read what changed before merging. Find the changelog, in order:
   a. gh release view v<new-version> --repo <pkg-owner/pkg-repo> (and any intermediate tags).
   b. The package repo's CHANGELOG.md between the two versions.
   c. The GitHub compare view if neither exists.
   - Changelog found and nothing in it suggests a breaking change (a removal, a deprecation, a changed default, a migration step) -> go to AUTO-APPROVE + MERGE.
   - No changelog found, or it suggests a breaking change -> go to ASK THE USER.
