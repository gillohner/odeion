# Usage insights

Usage insights count which pages and buttons people use on your node, which buttons they are shown
and pass by, and where things go wrong (an error, a slow page, a button that does nothing, going
straight back). They exist to guide changes to the app: what to make easier, and what nobody seems
to need.

> **Status:** this release has the node's side only: the switch, people's choices, storage and the
> summary. The browser part (the notice, the Settings pages described below, and the recording
> itself) comes next. Until then nothing is recorded, even with the switch on.

They can't explain why someone did something. A button that is shown and not used is not disliked:
it may simply not be needed that day. So the summary only gives a use rate once a button was really
shown often enough, to enough browsers.

## Turning it on

Usage insights are **off** on a new node and stay off until the owner turns them on, in
**Settings › This node › Usage insights**. The switch is kept across restarts.

Once it is on, every signed-in browser takes part automatically, both the owner's and listeners'.
Each person sees a short notice that says what is recorded and how to stop. Turning it off ends
every browser's part at once and keeps what is stored; **Clear all** is a separate button.

Party guests are not covered yet. The summary says `guests: not instrumented` until they are.

## Each person's own controls

In **Settings › You › Privacy › Usage insights**, and from the help menu:

- **Stop for me** applies at once, on every browser you are signed in with, even if the node can't
  save it right then: it says so, and saves it as soon as it can (a restart before that would
  forget it). A Clear at the same moment never refuses it. What your browser sent and the node had
  not written yet is not written, and nothing it was still sending lands afterwards. You can turn it
  back on there.
- **Clear mine** deletes the raw events still linked to you: those of your open browsers and of
  up to 200 closed within the last day. Older events are no longer linked to anyone (see below)
  and go when they expire. The daily counts (how many pages, buttons and page loads a day, with no
  alias) are not linked to anyone and stay their 90 days, also after Clear mine or being removed
  from the node. On a node with one listener, those counts are mostly that person's.

Stop and Clear are separate: Stop does not delete, and Clear does not stop. Anything your browser
was still sending when you pressed Clear never lands after it.

## What is recorded

Only fixed categories:

| What | Kept |
|---|---|
| Pages | Which page (Albums, Search, Settings…), never which album, search or person |
| Page loading and requests | The kind of page or request, whether it worked, and a coarse duration |
| Buttons | Which kind of button (Play, Shuffle, a menu's Add to queue…) and how (click, key, swipe) |
| Shown and used | Per page visit, which buttons were really on screen (at least half of it, for a second, not covered or disabled, the tab in front) and which were used |
| Trouble | Error messages shown (only their kind), Try again, repeated clicks, a click on something that does nothing, going straight back, a page still loading or empty |
| Script errors | The built-in error class only, never the message |
| Browser | Phone, tablet or desktop, and whether it is the owner's or a listener's |

**Never recorded:** anything you type (searches, tags, names, comments, passwords, keys), song,
album, artist, playlist or tag names, addresses or links, ids of songs or people, error messages or
stacks, what is on screen, screenshots, audio, keystrokes, IP addresses, the browser's name,
cookies or sign-ins. Nothing other people do on a zone is recorded as yours.

## Who it is, and who it isn't

Each page load gets a random alias. Which person an alias belongs to is known only in the node's
memory: for the alias's life (it ends after 30 minutes without events, or 12 hours) and one day
after, so Clear mine can find it. A restart forgets it. The stored events carry the alias, owner or
listener, and phone, tablet or desktop. No key, token or address.

This is pseudonymous, not anonymous. On a node with one owner, "owner" points to the owner's own
browsers, and on a node with few listeners, "listener" with "phone" may point to one person. Someone
who also knows when people played what (the owner sees zones and queues) could match an alias's
times to a person. To make that harder, times are kept to the minute only, and an alias lasts 12
hours at most. People's Stop choices are stored under a hashed key; someone holding the file who
knows the listeners' keys can still tell who chose Stop.

When the owner removes a listener, their choice and the events still linked to them are deleted.
If the file can't take that right then, the node tries again until it can (the owner's status
counts the people waiting for it). Should the node restart before that, the events are no longer
linked to anyone and go with retention, and the old choice stays in the file under its hashed key:
if the same person is invited again it applies to them again. It is never more than they chose, a
Stop or the default.

## Where it is kept, and for how long

In `usage.sqlite` in the node's data folder, readable only by the node's user (0600). The file is made when the owner first turns usage insights on, or when someone first records a choice (Stop, or that they read the notice); until the switch is on it holds only those choices. It is never in
a backup or a restore (Settings › Backup, and the bundle's `backup.sh` and `restore.sh`, which also
leaves out a copy in a backup made before), and never copied to a homeserver. Copying the whole data
folder by hand copies it too: leave `usage.sqlite` out ([backup guide](backup-and-update.md)).

- Raw events: 14 days.
- Daily counts: 90 days.
- At most 400 000 events and 64 MiB on disk. A write that would make the file bigger than that
  first makes room (the oldest day goes, or the oldest tenth when only one day is left), and what
  still doesn't fit is counted as evicted, so the file itself never grows past the cap; while a
  write runs, SQLite's journal is extra, and an event over the count is gone at the end of that
  write. Free space is given back when most of the file is free. That needs room on the disk for a
  copy of the file: when it fails, it is tried again an hour later, and events fill the free space
  meanwhile.
- Times are kept to the minute.
- One person can have at most 8 browsers taking part at once and open 30 a minute, so nobody can
  crowd out anyone else.

Events can be dropped on the way: the limits per browser (120 a minute), per person (240) and for
the node (3000), a full queue, a browser that was offline. Drops are counted, so the summary never
reads as complete when it isn't.

## Reading it

- **Summary** (Settings, or `GET /v1/node/usage/summary`): counts only, no aliases or rows.
- **Download** (`GET /v1/node/usage/export`): the raw events of the last 14 days, each browser
  renamed `s1`, `s2`… for that download only. Review it before you share it. A downloaded copy is
  outside the node's limits: delete it when you no longer need it.
- **On the node itself:** `odeiond usage summary` and `odeiond usage export` read the file and print
  JSON. They change nothing.

Nothing is sent anywhere by itself. There is no third-party service.

## Limits of what it says

- Shown and not used is not a dislike. A use rate needs at least 30 visits that showed the button,
  from at least 5 browsers; below that the summary says `insufficient`.
- Trouble signals show where to look, not why.
- One very busy person can fill much of the raw-event storage and weigh heavily in the summary.
  Read the per-alias counts next to the rates.
- In a download, browsers get names drawn afresh for that download (`s1`, `s2`… in a random
  order, never the same across downloads), and events are ordered by time and then by those names.
  Each browser's own events keep their order.
- Usage insights can't replace asking people. Use the feedback entry (the "?" button) for that.
