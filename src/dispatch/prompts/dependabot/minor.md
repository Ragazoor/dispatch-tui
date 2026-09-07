This is a minor bump, so read what changed before merging. Find the changelog, in order:
   a. gh release view v<new-version> --repo <pkg-owner/pkg-repo> (and any intermediate tags).
   b. The package repo's CHANGELOG.md between the two versions.
   c. The GitHub compare view if neither exists.
   Scan the release notes for these tokens (case-insensitive): BREAKING, breaking change, removed, deprecat, incompatible, migration, major rewrite.
   - Changelog found AND no token matched -> go to AUTO-APPROVE + MERGE.
   - No changelog found OR any token matched -> go to ASK THE USER.
