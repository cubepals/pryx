# Running K19 to K27 unattended

How one task takes the any-host plan (docs/any-host.md) from K19 to K27 with nobody watching, for
as many days as it needs. A task is started with one line: "Do what PLAN-ANY-HOST.md says." A task
that stops is started again with the same line and continues where the files say it was.

```
The work

Run PLAN-K19.md, PLAN-K20.md, PLAN-K21.md, PLAN-K22.md, PLAN-K23.md, PLAN-K24.md, PLAN-K25.md,
PLAN-K26.md and PLAN-K27.md, one by one, in that order. Read AGENTS.md first, above all "How
pryx decides", then "How to run this plan" in PLAN.md, then docs/any-host.md, then each brief as
you reach it. Where this file and PLAN.md differ, this file holds.

The state is in the repository

- Work on one branch, `any-host`, made from main. If it exists, continue on it.
- Before anything else, create docs/any-host-progress.md if it isn't there: one row per
  milestone with its status (not started, in progress, done, done except), the commit that
  finished it, the CI run that proved it, what ran, what did not, what is parked and why, and
  what is left. Update it and commit whenever a step finishes.
- At the start of every milestone, and whenever you are unsure where you are, re-read AGENTS.md,
  docs/any-host-progress.md, docs/decisions.md and that milestone's brief. Trust them over
  anything you remember. A new task with no history must be able to continue from them alone.
- Commit small and often, and push the branch after every commit that builds. Nothing that
  matters exists only on this machine.
- Commit and push each step that builds, at least every thirty minutes. Checkpoints preserve
  incomplete milestones honestly; a machine failure must not lose a larger interval of work.

CI is the proof

- Everything a brief's "Done when" claims is proved by a test that CI runs on the real thing: a
  real Docker, a real rootless engine, a real Podman, a real /dev/kvm on a hosted runner, a
  default `docker run`, a booted distribution. A claim only this machine checked is not proved.
- You may add the CI jobs, runners' setup steps and the scheduled matrix workflow the briefs
  call for. Every workflow change passes actionlint before it is pushed. Write in the progress
  file how many minutes each addition costs a pull request. A new workflow that can't be started
  by hand before it is on main also runs on pushes to `any-host`, and says so in a comment.
- After each push, wait for the branch's CI run, read every failure in its logs, fix the cause
  and push again. A milestone is done when CI is green on its last commit; record that run in
  the progress file. Then begin the next.
- Keep raw output in CI artifacts. Under docs/checks/measurements commit only inputs read by
  code or checks and one short summary per measurement: what ran, decisive numbers, source
  commit and CI URL. Remove raw logs/dumps and update every reference when replacing them.
- Each CI inspection also reads PR 21 scanner comments through GitHub's API. Fix new findings
  as they appear; test cryptographic challenges are generated at runtime, preserving assertions.
- If you cannot push, or cannot read CI's results from here, write that once in the progress
  file, run everything that can run on this machine, and keep going.

Never block yourself

- Never ask and never wait for a person. Take the simplest decision that keeps every rule,
  record it in docs/decisions.md (one line, with the milestone), and go on.
- A tool that is missing is installed. A registry, mirror or download that fails is retried with
  a longer wait and then replaced by another source at the same pinned digest; an outage is
  never a reason to stop.
- When a side piece's failure has survived three different attempts, park that side piece:
  write what fails, the attempts and evidence in the progress file; leave it off with a named
  refusal and continue independent work. "Done except" names only these side pieces. The
  main deliverable of a milestone is never parked: choose another approach and continue until
  it works and CI proves it. In K23 the relocatable bundle and ordinary-user browser/Python/
  Node kernels are the main deliverable, as explicitly corrected by the user.
- After K27, return to every parked piece once, with what was learned since.
- Stop only when every milestone is done or done except, and every parked piece has had its
  second try.

Never, whatever it would unblock

- Skip, disable or loosen a test of something that exists; claim a check passed that did not
  run; weaken a guarantee, a hardening or a refusal to get to green.
- Spend money: no paid runners, services or accounts.
- Push to main, force-push, push a tag, raise the version, or change the repository's settings,
  secrets or release keys.
- Rename or remove anything in the node protocol, the public API or the edge protocol.

The end

Update docs/any-host-progress.md a last time and leave `any-host` pushed and green.
After all builds and milestone verification, land one pull request per milestone, in order,
not one combined squash; keep commits confined to one milestone so the split is clean. Open no
pull request during the build. The last
message lists, for each milestone: what was built, the CI run that proved it, what did not run,
what is parked and why, and every decision recorded.
```
