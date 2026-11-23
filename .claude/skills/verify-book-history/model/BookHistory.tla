---- MODULE BookHistory ----
EXTENDS Integers, Sequences, FiniteSets, TLC

CONSTANTS MaxAppends, MaxCrashes, MaxPowerCuts, MaxExternal, MaxFlushFails, TornUnsynced, MaxPrunes, MaxHolds, Mutant

Entries == {"a", "b"}
Contents == [a : {1, 2}, b : {0, 1}]
Original == [a |-> 1, b |-> 0]
NoChanges == [e \in {} |-> 0]
NoRec == [kind |-> "none", before |-> Original, after |-> Original, changes |-> NoChanges, carried |-> NoChanges, id |-> 0]
EmptyIx == [end |-> 0, blobs |-> [k \in {} |-> 0], recs |-> <<>>]
Bad == -1
Garbled == -2
MaxChain == 3

BlobFrame(k) == [kind |-> "blob", key |-> k, base |-> 0, from |-> 0, rec |-> NoRec, st |-> "ok", dur |-> "unsynced"]
TextFrame(k, b, t) == [kind |-> "text", key |-> k, base |-> b, from |-> t, rec |-> NoRec, st |-> "ok", dur |-> "unsynced"]
RecFrame(r) == [kind |-> "rec", key |-> 0, base |-> 0, from |-> 0, rec |-> r, st |-> "ok", dur |-> "unsynced"]
Partial == [kind |-> "rec", key |-> 0, base |-> 0, from |-> 0, rec |-> NoRec, st |-> "gone", dur |-> "unsynced"]

VARIABLES
    disk, bookCands, log,
    pc, view, idx, target, cur,
    nextId, appends, crashes, cuts, externals, fails,
    warned, vouched, clean,
    oldLog, pending, prunes, pruned, untrusted, held, trusted

vars == <<disk, bookCands, log, pc, view, idx, target, cur,
          nextId, appends, crashes, cuts, externals, fails, warned, vouched, clean,
          oldLog, pending, prunes, pruned, untrusted, held, trusted>>

Last(s) == s[Len(s)]
Front(s) == SubSeq(s, 1, Len(s) - 1)

Intact(f) == f.st = "ok"

RECURSIVE ScanFrom(_, _)
ScanFrom(lg, ix) ==
    IF ix.end >= Len(lg) THEN ix
    ELSE LET at == ix.end + 1
             f == lg[at]
         IN CASE ~Intact(f) ->
                    IF Mutant # "stop_at_damage" /\ \E j \in (at + 1)..Len(lg) : Intact(lg[j])
                    THEN ScanFrom(lg, [ix EXCEPT !.end = at])
                    ELSE ix
              [] f.kind \in {"blob", "text"} ->
                    ScanFrom(lg, [ix EXCEPT !.end = at,
                        !.blobs = IF f.key \in DOMAIN ix.blobs
                                  THEN [ix.blobs EXCEPT ![f.key] = Append(@, at)]
                                  ELSE ix.blobs @@ (f.key :> <<at>>)])
              [] OTHER -> ScanFrom(lg, [ix EXCEPT !.end = at, !.recs = Append(ix.recs, f.rec)])

Refresh(lg, ix) == ScanFrom(lg, IF ix.end > Len(lg) THEN EmptyIx ELSE ix)

Min(S) == CHOOSE x \in S : \A y \in S : x <= y

Copies(ix, k) ==
    IF k \notin DOMAIN ix.blobs THEN <<>>
    ELSE IF Mutant = "first_copy" THEN SubSeq(ix.blobs[k], 1, 1)
    ELSE ix.blobs[k]

FrameOk(lg, p, k) ==
    /\ p <= Len(lg)
    /\ lg[p].kind \in {"blob", "text"}
    /\ lg[p].key = k
    /\ lg[p].st = "ok"

Indexed(lg, ix, k) == \E i \in 1..Len(Copies(ix, k)) : FrameOk(lg, Copies(ix, k)[i], k)

RECURSIVE ReadBack(_, _, _, _), ReadAt(_, _, _, _, _), FirstRead(_, _, _, _, _, _)
ReadAt(lg, ix, p, k, depth) ==
    IF ~FrameOk(lg, p, k) THEN Bad
    ELSE LET f == lg[p]
         IN IF f.base = 0 THEN k
            ELSE IF depth >= MaxChain THEN Bad
            ELSE LET t == ReadBack(lg, ix, f.base, depth + 1)
                 IN IF t = Bad THEN Bad
                    ELSE IF t = f.from THEN k
                    ELSE IF Mutant = "wrong_base_unchecked" THEN Garbled
                    ELSE Bad

FirstRead(lg, ix, cs, i, k, depth) ==
    IF i > Len(cs) THEN Bad
    ELSE LET r == ReadAt(lg, ix, cs[i], k, depth)
         IN IF r # Bad THEN r ELSE FirstRead(lg, ix, cs, i + 1, k, depth)

ReadBack(lg, ix, k, depth) == FirstRead(lg, ix, Copies(ix, k), 1, k, depth)

Pick(lg, ix, k) ==
    LET cs == Copies(ix, k)
        good == {i \in 1..Len(cs) : ReadAt(lg, ix, cs[i], k, 0) # Bad}
        ok == {i \in 1..Len(cs) : FrameOk(lg, cs[i], k)}
    IN cs[IF good # {} THEN Min(good) ELSE IF ok # {} THEN Min(ok) ELSE 1]

ReadText(lg, ix, k) == IF k = 0 THEN 0 ELSE ReadBack(lg, ix, k, 0)

Readable(lg, ix, k) == ReadText(lg, ix, k) # Bad

RECURSIVE PopFor(_, _)
PopFor(pts, before) ==
    IF /\ Len(pts) > 0
       /\ Last(pts).rec.after # before
       /\ (Last(pts).rec.before = before \/ Mutant = "pop_any")
    THEN PopFor(Front(pts), before)
    ELSE pts

RECURSIVE Build(_, _, _)
Build(recs, i, pts) ==
    IF i > Len(recs) THEN pts
    ELSE Build(recs, i + 1,
               Append(IF Mutant = "no_pop" THEN pts ELSE PopFor(pts, recs[i].before),
                      [id |-> i, rec |-> recs[i]]))

Points(recs, now) ==
    LET built == Build(recs, 1, <<>>)
    IN IF Mutant = "no_tail_pop" THEN built ELSE PopFor(built, now)

GapBefore(pts, k) == k > 1 /\ pts[k - 1].rec.after # pts[k].rec.before

All(rec) == rec.changes @@ rec.carried

RECURSIVE Walk(_, _, _, _)
Walk(pts, k, j, known) ==
    IF k > Len(pts) THEN [known |-> known, broke |-> FALSE]
    ELSE IF k > j /\ GapBefore(pts, k) /\ Mutant # "ignore_gaps"
    THEN [known |-> known, broke |-> TRUE]
    ELSE Walk(pts, k + 1, j,
              IF Mutant = "last_wins" THEN All(pts[k].rec) @@ known ELSE known @@ All(pts[k].rec))

StateAt(pts, now, j) ==
    LET w == Walk(pts, j, j, NoChanges)
        gapAtEnd == Last(pts).rec.after # now /\ Mutant # "ignore_gaps"
    IN [known |-> w.known, complete |-> ~w.broke /\ ~gapAtEnd, before |-> pts[j].rec.before]

Restored(s, now) == [e \in Entries |-> IF e \in DOMAIN s.known THEN s.known[e] ELSE now[e]]

Views ==
    (IF pc = "idle" /\ view = disk THEN {Refresh(log, idx)} ELSE {})
    \cup {Refresh(log, EmptyIx)}

Shown(ix) == Points(ix.recs, disk)

Trusted(pts, j) == \A k \in j..Len(pts) : pts[k].rec.id \notin untrusted /\ \E v \in vouched : v.id = pts[k].rec.id

VersionExact ==
    \A ix \in Views : LET pts == Shown(ix) IN
        \A j \in 1..Len(pts) : LET s == StateAt(pts, disk, j) IN
            \A e \in DOMAIN s.known :
                /\ s.known[e] = s.before[e]
                /\ ReadText(log, ix, s.known[e]) \in {s.before[e], Bad}

RestoreExact ==
    \A ix \in Views : LET pts == Shown(ix) IN
        \A j \in 1..Len(pts) : LET s == StateAt(pts, disk, j) IN
            s.complete => Restored(s, disk) = s.before

Restorable ==
    \A ix \in Views : LET pts == Shown(ix) IN
        \A j \in 1..Len(pts) : LET s == StateAt(pts, disk, j) IN
            Trusted(pts, j) =>
                \A e \in DOMAIN s.known : s.known[e] # 0 => Readable(log, ix, s.known[e])

NoPointLost ==
    \A ix \in Views : LET pts == Shown(ix) IN
        \A v \in vouched : v.id \in pruned \/ v.before = disk \/ \E j \in 1..Len(pts) : pts[j].rec.before = v.before

NoFalseGap ==
    clean =>
        \A ix \in Views : LET pts == Shown(ix) IN
            \A j \in 1..Len(pts) : StateAt(pts, disk, j).complete

Identity(rec) ==
    [kind |-> rec.kind, at |-> rec.id, before |-> rec.before, entries |-> DOMAIN rec.changes,
     after |-> IF Mutant \in {"id_from_after", "id_from_after_verified"} THEN rec.after ELSE Original]

Handle(pts, j) == IF Mutant = "position_ids" THEN pts[j].id ELSE Identity(pts[j].rec)

HeldExact ==
    \A h \in held : \A ix \in Views : LET pts == Shown(ix) IN
        \A j \in 1..Len(pts) : Handle(pts, j) = h.handle => pts[j].rec.id = h.id

HeldFound ==
    \A h \in held : \A ix \in Views : LET pts == Shown(ix) IN
        (\E j \in 1..Len(pts) : pts[j].rec.id = h.id) => \E j \in 1..Len(pts) : Handle(pts, j) = h.handle

RECURSIVE Ordered(_)
Ordered(S) ==
    IF S = {} THEN <<>>
    ELSE LET m == CHOOSE x \in S : \A y \in S : x <= y IN <<m>> \o Ordered(S \ {m})

BlobSeq(S) == LET o == Ordered(S) IN [i \in 1..Len(o) |-> BlobFrame(o[i])]

Kept(ix) == SubSeq(log, 1, ix.end)

SyncedAll(lg) == [i \in 1..Len(lg) |-> IF lg[i].dur = "unsynced" THEN [lg[i] EXCEPT !.dur = "ok"] ELSE lg[i]]

Unsynced(lg) == {i \in 1..Len(lg) : lg[i].dur = "unsynced"}

LostSome(lg, S) ==
    [i \in 1..Len(lg) |->
        IF i \in S THEN [lg[i] EXCEPT !.dur = "lost"]
        ELSE IF lg[i].dur = "unsynced" THEN [lg[i] EXCEPT !.dur = "ok"]
        ELSE lg[i]]

AfterFail(lg, start, S) == LostSome(SubSeq(lg, 1, start), S)

Init ==
    /\ disk = Original
    /\ bookCands = {Original}
    /\ log = <<>>
    /\ pc = "idle"
    /\ view = Original
    /\ idx = EmptyIx
    /\ target = Original
    /\ cur = 0
    /\ nextId = 1
    /\ appends = 0
    /\ crashes = 0
    /\ cuts = 0
    /\ externals = 0
    /\ fails = 0
    /\ warned = {}
    /\ vouched = {}
    /\ clean = TRUE
    /\ oldLog = <<>>
    /\ pending = FALSE
    /\ prunes = 0
    /\ pruned = {}
    /\ untrusted = {}
    /\ held = {}
    /\ trusted = {}

Open ==
    /\ \/ pc = "closed"
       \/ pc = "idle" /\ view # disk
    /\ pc' = "idle"
    /\ view' = disk
    /\ idx' = EmptyIx
    /\ bookCands' = {disk}
    /\ trusted' = {}
    /\ UNCHANGED <<disk, log, target, cur, nextId, appends, crashes, cuts, externals, fails, warned, vouched, clean, oldLog, pending, prunes, pruned, untrusted, held>>

Reusable(ix) == IF Mutant = "trust_any" THEN DOMAIN ix.blobs ELSE trusted \cap DOMAIN ix.blobs

Written(lg, from) == {lg[i].key : i \in {i \in (from + 1)..Len(lg) : lg[i].kind \in {"blob", "text"}}}

Stage(c) ==
    /\ pc = "idle"
    /\ view = disk
    /\ appends < MaxAppends
    /\ c # disk
    /\ LET ix == Refresh(log, idx)
           ch == {e \in Entries : c[e] # disk[e]}
           rec == [kind |-> "save", before |-> disk, after |-> c,
                   changes |-> [e \in ch |-> disk[e]], carried |-> NoChanges, id |-> nextId]
           keys == {disk[e] : e \in ch} \ ({0} \cup Reusable(ix))
           fr == (IF Mutant = "skip_blobs" THEN <<>> ELSE BlobSeq(keys)) \o <<RecFrame(rec)>>
       IN /\ idx' = ix
          /\ \/ /\ log' = Kept(ix) \o fr
                /\ pc' = "staged"
                /\ target' = c
                /\ cur' = nextId
                /\ UNCHANGED crashes
             \/ /\ crashes < MaxCrashes
                /\ \E k \in 0..(Len(fr) - 1) : log' = Kept(ix) \o SubSeq(fr, 1, k) \o <<Partial>>
                /\ pc' = "closed"
                /\ crashes' = crashes + 1
                /\ UNCHANGED <<target, cur>>
    /\ nextId' = nextId + 1
    /\ appends' = appends + 1
    /\ UNCHANGED <<disk, bookCands, view, cuts, externals, fails, warned, vouched, clean, oldLog, pending, prunes, pruned, untrusted, held, trusted>>

SyncHistory ==
    /\ pc = "staged"
    /\ pc' = "synced"
    /\ \/ /\ log' = SyncedAll(log)
          /\ pending' = (pending /\ Mutant = "no_dir_sync")
          /\ trusted' = trusted \cup Written(log, idx.end)
          /\ UNCHANGED <<fails, warned, clean, crashes>>
       \/ /\ fails < MaxFlushFails
          /\ \E S \in SUBSET Unsynced(log) :
                /\ S # {}
                /\ log' = AfterFail(log, idx.end, S)
          /\ fails' = fails + 1
          /\ warned' = warned \cup {cur}
          /\ clean' = FALSE
          /\ UNCHANGED <<pending, crashes, trusted>>
    /\ UNCHANGED <<disk, bookCands, view, idx, target, cur, nextId, appends, cuts, externals, vouched, oldLog, prunes, pruned, untrusted, held>>

KilledBeforeCut ==
    /\ pc = "staged"
    /\ fails < MaxFlushFails
    /\ crashes < MaxCrashes
    /\ \E S \in SUBSET Unsynced(log) :
          /\ S # {}
          /\ log' = LostSome(log, S)
    /\ fails' = fails + 1
    /\ crashes' = crashes + 1
    /\ clean' = FALSE
    /\ pc' = "closed"
    /\ UNCHANGED <<disk, bookCands, view, idx, target, cur, nextId, appends, cuts, externals, warned, vouched, oldLog, pending, prunes, pruned, untrusted, held, trusted>>

Rename ==
    /\ pc = "synced"
    /\ IF disk = view
       THEN /\ disk' = target
            /\ bookCands' = bookCands \cup {target}
            /\ pc' = "renamed"
       ELSE /\ pc' = "idle"
            /\ UNCHANGED <<disk, bookCands>>
    /\ UNCHANGED <<log, view, idx, target, cur, nextId, appends, crashes, cuts, externals, fails, warned, vouched, clean, oldLog, pending, prunes, pruned, untrusted, held, trusted>>

Finish ==
    /\ pc = "renamed"
    /\ bookCands' = {disk}
    /\ view' = target
    /\ pc' = "idle"
    /\ vouched' = IF cur \in warned THEN vouched ELSE vouched \cup {[id |-> cur, before |-> view]}
    /\ UNCHANGED <<disk, log, idx, target, cur, nextId, appends, crashes, cuts, externals, fails, warned, clean, oldLog, pending, prunes, pruned, untrusted, held, trusted>>

Snapshot ==
    /\ pc = "idle"
    /\ view = disk
    /\ appends < MaxAppends
    /\ LET ix == Refresh(log, idx)
           rec == [kind |-> "named", before |-> disk, after |-> disk, changes |-> NoChanges, carried |-> NoChanges, id |-> nextId]
           written == Kept(ix) \o <<RecFrame(rec)>>
       IN /\ idx' = ix
          /\ \/ /\ log' = SyncedAll(written)
                /\ vouched' = vouched \cup {[id |-> nextId, before |-> disk]}
                /\ pending' = (pending /\ Mutant = "no_dir_sync")
                /\ UNCHANGED <<fails, warned, clean>>
             \/ /\ fails < MaxFlushFails
                /\ \E S \in SUBSET Unsynced(written) :
                      /\ S # {}
                      /\ log' = AfterFail(written, ix.end, S)
                /\ fails' = fails + 1
                /\ warned' = warned \cup {nextId}
                /\ clean' = FALSE
                /\ UNCHANGED <<vouched, pending>>
    /\ nextId' = nextId + 1
    /\ appends' = appends + 1
    /\ UNCHANGED <<disk, bookCands, pc, view, target, cur, crashes, cuts, externals, oldLog, prunes, pruned, untrusted, held, trusted>>

Crash ==
    /\ pc \in {"idle", "staged", "synced", "renamed"}
    /\ crashes < MaxCrashes
    /\ pc' = "closed"
    /\ crashes' = crashes + 1
    /\ UNCHANGED <<disk, bookCands, log, view, idx, target, cur, nextId, appends, cuts, externals, fails, warned, vouched, clean, oldLog, pending, prunes, pruned, untrusted, held, trusted>>

Settle(f) ==
    CASE f.st # "ok" -> {[f EXCEPT !.dur = "ok"]}
      [] f.dur = "lost" -> {[f EXCEPT !.st = s, !.dur = "ok"] : s \in {"body", "gone"}}
      [] f.dur = "unsynced" ->
            {[f EXCEPT !.st = s, !.dur = "ok"] : s \in (IF TornUnsynced THEN {"ok", "body", "gone"} ELSE {"ok"})}
      [] OTHER -> {f}

RECURSIVE Choices(_)
Choices(s) == IF s = <<>> THEN {<<>>} ELSE {Append(t, f) : t \in Choices(Front(s)), f \in Settle(Last(s))}

FirstUnsynced(lg) == IF Unsynced(lg) = {} THEN Len(lg) + 1 ELSE CHOOSE i \in Unsynced(lg) : \A j \in Unsynced(lg) : i <= j

AfterCut(lg) == UNION {Choices(SubSeq(lg, 1, k)) : k \in (FirstUnsynced(lg) - 1)..Len(lg)}

PowerCut ==
    /\ cuts < MaxPowerCuts
    /\ \E d \in bookCands, lg \in AfterCut(log) \cup (IF pending THEN AfterCut(oldLog) ELSE {}) :
          /\ disk' = d
          /\ bookCands' = {d}
          /\ log' = [i \in 1..Len(lg) |-> lg[i]]
    /\ pc' = "closed"
    /\ cuts' = cuts + 1
    /\ pending' = FALSE
    /\ UNCHANGED <<view, idx, target, cur, nextId, appends, crashes, externals, fails, warned, vouched, clean, oldLog, prunes, pruned, untrusted, held, trusted>>

Evict ==
    /\ \E i \in 1..Len(log), s \in {"body", "gone"} :
          /\ log[i].dur = "lost"
          /\ log[i].st = "ok"
          /\ log' = [log EXCEPT ![i].st = s]
    /\ UNCHANGED <<disk, bookCands, pc, view, idx, target, cur, nextId, appends, crashes, cuts, externals, fails, warned, vouched, clean, oldLog, pending, prunes, pruned, untrusted, held, trusted>>

OutsideWrite ==
    /\ externals < MaxExternal
    /\ \E d \in Contents \ {disk} :
          /\ disk' = d
          /\ bookCands' = {d}
    /\ externals' = externals + 1
    /\ clean' = FALSE
    /\ UNCHANGED <<log, pc, view, idx, target, cur, nextId, appends, crashes, cuts, fails, warned, vouched, oldLog, pending, prunes, pruned, untrusted, held, trusted>>


Verified == Mutant \notin {"fold_gaps", "fold_last", "drop_named", "drop_newest", "prune_skip_blobs", "prune_no_sync",
                            "unwritten_base", "wrong_base", "wrong_base_unchecked", "stale_base", "id_from_after"}

TextsVerified == Verified /\ Mutant # "verify_skips_texts"

UnwrittenBase == Mutant \in {"unwritten_base", "unwritten_base_verified", "verify_skips_texts"}
WrongBase == Mutant \in {"wrong_base", "wrong_base_verified", "wrong_base_unchecked"}
StaleBase == Mutant \in {"stale_base", "stale_base_verified"}

KeepPoint(pts, k, c) ==
    \/ pts[k].rec.kind = "named" /\ Mutant # "drop_named"
    \/ pts[k].rec.id >= c
    \/ k = Len(pts) /\ Mutant # "drop_newest"

MayDrop(pts, k, c) == pts[k].rec.kind = "save" /\ pts[k].rec.id < c /\ k # Len(pts)

Folded(into, from, seen) ==
    LET fresh == IF Mutant = "fold_last" THEN DOMAIN All(from) ELSE DOMAIN All(from) \ seen
        add == [e \in fresh |-> All(from)[e]]
    IN [into EXCEPT !.carried = IF Mutant = "fold_last" THEN add @@ into.carried ELSE into.carried @@ add,
                    !.after = from.after]

RECURSIVE Plan(_, _, _, _, _, _)
Plan(pts, k, c, recs, seen, folding) ==
    IF k > Len(pts) THEN recs
    ELSE IF KeepPoint(pts, k, c)
    THEN Plan(pts, k + 1, c, Append(recs, pts[k].rec), DOMAIN All(pts[k].rec), TRUE)
    ELSE LET go == folding /\ (~GapBefore(pts, k) \/ Mutant \in {"fold_gaps", "fold_gaps_verified"})
         IN IF go /\ Len(recs) > 0
            THEN Plan(pts, k + 1, c, [recs EXCEPT ![Len(recs)] = Folded(Last(recs), pts[k].rec, seen)],
                      seen \cup DOMAIN All(pts[k].rec), TRUE)
            ELSE Plan(pts, k + 1, c, recs, seen, go)

Cited(rec) == {All(rec)[e] : e \in DOMAIN All(rec)} \ {0}

Durable(f) == IF Mutant = "prune_no_sync" THEN f ELSE [f EXCEPT !.dur = "ok"]

Linked(links, k) == \E i \in 1..Len(links) : links[i].key = k

LinkEntry(rec, e, st) ==
    IF e \notin DOMAIN All(rec) THEN st
    ELSE LET k == All(rec)[e]
         IN [newer |-> [st.newer EXCEPT ![e] = k],
             links |-> IF k = 0 \/ Linked(st.links, k) THEN st.links
                       ELSE Append(st.links, [key |-> k, entry |-> e, base |-> st.newer[e]])]

RECURSIVE LinkFrom(_, _, _)
LinkFrom(recs, i, st) ==
    IF i = 0 THEN st
    ELSE LinkFrom(recs, i - 1, LinkEntry(recs[i], "b", LinkEntry(recs[i], "a", st)))

Links(recs) ==
    LET links == LinkFrom(recs, Len(recs), [newer |-> [e \in Entries |-> 0], links |-> <<>>]).links
    IN [i \in 1..Len(links) |->
           [links[i] EXCEPT !.base = IF Linked(SubSeq(links, 1, i - 1), @) THEN @ ELSE 0]]

Recompute(f, b, k, D) ==
    /\ f.base # b
    /\ IF f.base # 0 THEN ~StaleBase ELSE k \in D

NewText(k, b, D) ==
    IF b # 0 /\ k \in D THEN TextFrame(k, b, IF WrongBase THEN k ELSE b) ELSE TextFrame(k, 0, 0)

RECURSIVE CompactFrom(_, _, _, _, _)
CompactFrom(links, i, ix, D, st) ==
    IF i > Len(links) THEN st
    ELSE LET k == links[i].key
             b == IF links[i].base \in st.placed THEN links[i].base ELSE 0
             next == IF k \notin DOMAIN ix.blobs THEN st
                     ELSE IF ~Readable(log, ix, k)
                     THEN IF log[Pick(log, ix, k)].kind = "text" THEN [st EXCEPT !.lost = @ \cup {k}] ELSE st
                     ELSE IF Recompute(log[Pick(log, ix, k)], b, k, D)
                     THEN [st EXCEPT !.placed = @ \cup {k}, !.texts = @ @@ (k :> NewText(k, b, D))]
                     ELSE [st EXCEPT !.placed = @ \cup {k}]
         IN CompactFrom(links, i + 1, ix, D, next)

Compact(links, ix, D) == CompactFrom(links, 1, ix, D, [placed |-> {}, texts |-> [k \in {} |-> 0], lost |-> {}])

Squeezable(links) == SUBSET {links[i].key : i \in {i \in 1..Len(links) : links[i].base # 0}}

CopyOf(k, ix, cp) == IF k \in DOMAIN cp.texts THEN cp.texts[k] ELSE [log[Pick(log, ix, k)] EXCEPT !.dur = "unsynced"]

RECURSIVE Rewrite(_, _, _, _, _, _)
Rewrite(recs, i, ix, cp, written, out) ==
    IF i > Len(recs) THEN out
    ELSE LET copy == {k \in Cited(recs[i]) \ (written \cup cp.lost) :
                         (k \in DOMAIN cp.texts \/ Indexed(log, ix, k)) /\ Mutant # "prune_skip_blobs"}
             keys == Ordered(copy)
             fr == [n \in 1..Len(keys) |-> CopyOf(keys[n], ix, cp)] \o <<RecFrame(recs[i])>>
         IN Rewrite(recs, i + 1, ix, cp, written \cup copy, out \o [n \in 1..Len(fr) |-> Durable(fr[n])])

PosOf(pts, id) == CHOOSE k \in 1..Len(pts) : pts[k].rec.id = id

Checked(old, new) ==
    /\ Len(new) = 0 <=> Len(old) = 0
    /\ Len(new) > 0 => Last(new).rec.id = Last(old).rec.id
    /\ \A k \in 1..Len(new) :
          /\ \E j \in 1..Len(old) : old[j].rec.id = new[k].rec.id
          /\ Identity(new[k].rec) = Identity(old[PosOf(old, new[k].rec.id)].rec)
          /\ StateAt(new, disk, k) = StateAt(old, disk, PosOf(old, new[k].rec.id))

SameTexts(recs, ix, new, newIx) ==
    \A k \in UNION {Cited(recs[i]) : i \in 1..Len(recs)} :
        /\ ReadText(new, newIx, k) = ReadText(log, ix, k)
        /\ k \in DOMAIN newIx.blobs => Readable(new, newIx, k)

Prune(c) ==
    /\ prunes < MaxPrunes
    /\ \/ pc = "closed"
       \/ pc = "idle" /\ view = disk
    /\ c \in 1..nextId
    /\ LET ix == IF pc = "idle" THEN Refresh(log, idx) ELSE Refresh(log, EmptyIx)
           pts == Points(ix.recs, disk)
           recs == Plan(pts, 1, c, <<>>, {}, FALSE)
           links == Links(IF UnwrittenBase THEN [i \in 1..Len(pts) |-> pts[i].rec] ELSE recs)
       IN \E D \in Squeezable(links) :
            LET cp == Compact(links, ix, D)
                new == Rewrite(recs, 1, ix, cp, {}, <<>>)
                newIx == Refresh(new, EmptyIx)
                newPts == Points(newIx.recs, disk)
            IN /\ Len(recs) < Len(ix.recs) \/ DOMAIN cp.texts # {}
               /\ ~Verified \/ Checked(pts, newPts)
               /\ ~TextsVerified \/ SameTexts(recs, ix, new, newIx)
               /\ log' = new
               /\ oldLog' = log
               /\ pending' \in BOOLEAN
               /\ pruned' = pruned \cup {v.id : v \in {v \in vouched :
                      LET same == {k \in 1..Len(pts) : pts[k].rec.before = v.before}
                      IN same # {} /\ \A k \in same : MayDrop(pts, k, c)}}
               /\ untrusted' = untrusted \cup
                     {pts[k].rec.id : k \in {k \in 1..Len(pts) : KeepPoint(pts, k, c) /\
                         \E m \in (k + 1)..Len(pts) :
                             /\ \A n \in (k + 1)..m : ~KeepPoint(pts, n, c) /\ ~GapBefore(pts, n)
                             /\ ~\E v \in vouched : v.id = pts[m].rec.id}}
               /\ idx' = EmptyIx
               /\ trusted' = trusted \cup Written(new, 0)
    /\ bookCands' = IF Mutant = "closed_no_sync" THEN bookCands ELSE {disk}
    /\ prunes' = prunes + 1
    /\ UNCHANGED <<disk, pc, view, target, cur, nextId, appends, crashes, cuts, externals, fails, warned, vouched, clean, held>>

Hold ==
    /\ Cardinality(held) < MaxHolds
    /\ pc = "idle"
    /\ view = disk
    /\ LET pts == Shown(Refresh(log, idx))
       IN \E j \in 1..Len(pts) : held' = held \cup {[id |-> pts[j].rec.id, handle |-> Handle(pts, j)]}
    /\ UNCHANGED <<disk, bookCands, log, pc, view, idx, target, cur, nextId, appends, crashes, cuts, externals, fails, warned, vouched, clean, oldLog, pending, prunes, pruned, untrusted, trusted>>

Next ==
    \/ Open
    \/ Hold
    \/ \E c \in Contents : Stage(c)
    \/ SyncHistory
    \/ KilledBeforeCut
    \/ Rename
    \/ Finish
    \/ Snapshot
    \/ Crash
    \/ PowerCut
    \/ Evict
    \/ OutsideWrite
    \/ \E c \in 1..(MaxAppends + 1) : Prune(c)

Spec == Init /\ [][Next]_vars
====
