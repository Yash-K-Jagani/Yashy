# Ownership

## Moves are explicit

**OWN-1.** Reading a value after it has been moved is
[E0500](../errors/E0500.md). The checker tracks the move at the point of the
transfer, so a use on one branch of a `switch` and a use on another are each
checked against the same state.

**ALIAS-1.** Copying an owning pointer without `move` is
[E1005](../errors/E1005.md); the program must say which binding owns the value.

**OWN-3.** Moving one field out of a struct does not let the other fields be
used as a whole; a partial move is [E1002](../errors/E1002.md).

## Release is a statement, not a destructor

`defer <expr>` schedules a release at the end of the enclosing scope.
`.free()` releases now. There is no `Drop` trait and no implicit release, so
the point at which storage goes away is visible in the source.

**OWN-2.** Leaving a live owned value without a release is a
[leak](../errors/E1000.md). The verifier's inventory reports which release
obligations it could prove.

## Arenas

`arena { }` gives one placement bit and one release at the block's end.
**OWN-4** forbids escape ([E1003](../errors/E1003.md)) and **OWN-5** forbids a
second release ([E1004](../errors/E1004.md)). An arena is not a general
allocator: it cannot be stored in a struct field that outlives the block.

## Threads and channels

**THREAD-1.** Closing a channel twice is [E1400](../errors/E1400.md).

**THREAD-2.** Reading or writing a closed channel is
[E1401](../errors/E1401.md).

**THREAD-3.** Two tasks writing the same location without synchronisation is a
[race](../errors/E1402.md). The rule is checked at the scope boundary, where a
task must have joined or cancelled its children before the scope is left.

## The verifier's role

The checker establishes the local facts: a value is moved once, a release is
present, a task does not escape. The verifier establishes the whole-program
facts and reports the missing proof rather than guessing. Its verdict is part
of `yashyc verify --json`; `build` and `run` refuse an unsound program.
