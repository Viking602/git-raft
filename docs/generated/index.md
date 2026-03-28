# Generated

- Runtime trace and run metadata are stored at `.git/git-raft/runs/<run-id>/`.
- External hooks can write their own payload captures or audit output to repo-local paths if the hook script chooses to do so.
- Real merge scenario captures can be copied to `docs/generated/real-merge/<scenario>/<timestamp>/`.
- Use this page to describe those fixed paths, their format, and retention rules.
- Generated output is not the source of truth.
- If a generated artifact should be kept long term, give it a clear name and document its purpose here.
- The real-merge helper scripts also copy run artifacts and summaries to `<sample-repo>/.git/git-raft-real-merge/<timestamp>/`.
