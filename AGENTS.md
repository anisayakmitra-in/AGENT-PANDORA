# Repository Guidelines & CI/CD Invariants

## CI/CD Pipeline Monitoring Invariant
Whenever changes are pushed to `origin` (including commits pushed to branches like `main` or release tags like `v*`):
1. **Do Not Terminate on `git push`**: A successful transport push is not sufficient to declare a task complete.
2. **Poll & Monitor Remote GitHub Actions**:
   - Query the GitHub Actions API or `gh run list` for runs triggered by the push commit SHA or tag.
   - Monitor in-progress runs until terminal conclusion (`success`, `failure`, `cancelled`).
3. **Verify All Matrix Runners**:
   - For multi-platform matrices (`ubuntu-latest`, `macos-*`, `windows-latest`), ensure every matrix arm passes.
4. **Immediate Forensic Inspection on Failure**:
   - If any workflow job or step fails, immediately inspect failed step logs, extract the panic/error traces, and report them.
5. **Report Status**:
   - Confirm pipeline green status with run IDs and URLs before declaring the release or milestone finished.
