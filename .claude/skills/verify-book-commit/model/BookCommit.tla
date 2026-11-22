---- MODULE BookCommit ----
EXTENDS Integers, FiniteSets, TLC

CONSTANTS Procs, MaxCommits, MaxCrashes, MaxPowerCuts, MaxExternal, DirSyncCanFail, Mutant

NONE == -1
TORN == -2
FREE == "free"
Original == 0
MaxVer == MaxCommits + MaxExternal
Vers == 0..MaxVer
NoLed == [head |-> NONE, pending |-> NONE]
Led(h, p) == [head |-> h, pending |-> p]

VARIABLES
    bookVis, bookCands, gen, synced, quiet,
    ledVis, ledDur, bakVis, bakDur, created, order,
    lock,
    pc, stamp, open, slot, mem, newv, revert, myGen, recheckGen,
    nextVer, commits, crashes, cuts, externals,
    missed, clobbered

fsVars == <<bookVis, bookCands, gen, synced, quiet, ledVis, ledDur, bakVis, bakDur, created, order>>
procVars == <<pc, stamp, open, slot, mem, newv, revert, myGen, recheckGen>>
countVars == <<nextVer, commits, crashes, cuts, externals>>
flagVars == <<missed, clobbered>>
vars == <<fsVars, lock, procVars, countVars, flagVars>>

Matches(b, cur) ==
    /\ ledVis[b] # NoLed
    /\ b \in bakVis
    /\ (ledVis[b].head = cur \/ ledVis[b].pending = cur)

FindValid(cur) ==
    IF \E b \in Vers : Matches(b, cur)
    THEN CHOOSE b \in Vers : Matches(b, cur) /\ \A c \in Vers : Matches(c, cur) => created[c] <= created[b]
    ELSE NONE

Recovered(l, cur) ==
    [b \in Vers |->
        IF l[b] # NoLed /\ l[b].pending # NONE
        THEN IF cur = l[b].pending THEN Led(cur, NONE)
             ELSE IF cur = l[b].head THEN Led(l[b].head, NONE)
             ELSE l[b]
        ELSE l[b]]

StampSame(p) == \A g \in (stamp[p] + 1)..gen : g \in quiet

HasAnySlot == bakVis # {} \/ \E b \in Vers : ledVis[b] # NoLed

Init ==
    /\ bookVis = Original /\ bookCands = {Original} /\ gen = 0 /\ synced = {Original} /\ quiet = {}
    /\ ledVis = [b \in Vers |-> NoLed] /\ ledDur = [b \in Vers |-> NoLed]
    /\ bakVis = {} /\ bakDur = {} /\ created = [b \in Vers |-> 0] /\ order = 0
    /\ lock = FREE
    /\ pc = [p \in Procs |-> "closed"]
    /\ stamp = [p \in Procs |-> 0]
    /\ open = [p \in Procs |-> NONE]
    /\ slot = [p \in Procs |-> NONE]
    /\ mem = [p \in Procs |-> NoLed]
    /\ newv = [p \in Procs |-> NONE]
    /\ revert = [p \in Procs |-> FALSE]
    /\ myGen = [p \in Procs |-> 0]
    /\ recheckGen = [p \in Procs |-> 0]
    /\ nextVer = 1 /\ commits = 0 /\ crashes = 0 /\ cuts = 0 /\ externals = 0
    /\ missed = FALSE /\ clobbered = FALSE

Goto(p, where) == pc' = [pc EXCEPT ![p] = where]

Abort(p) ==
    /\ lock' = FREE
    /\ Goto(p, "idle")

StoreLedger(b, l) ==
    /\ ledVis' = [ledVis EXCEPT ![b] = l]
    /\ ledDur' = [ledDur EXCEPT ![b] = l]

OpenBook(p) ==
    /\ pc[p] = "closed"
    /\ bookVis # TORN
    /\ stamp' = [stamp EXCEPT ![p] = gen]
    /\ open' = [open EXCEPT ![p] = bookVis]
    /\ slot' = [slot EXCEPT ![p] = NONE]
    /\ mem' = [mem EXCEPT ![p] = NoLed]
    /\ Goto(p, IF HasAnySlot THEN "o_recover" ELSE "idle")
    /\ UNCHANGED <<fsVars, lock, newv, revert, myGen, recheckGen, countVars, flagVars>>

OpenRecover(p) ==
    /\ pc[p] = "o_recover"
    /\ \/ /\ lock = FREE
          /\ bookCands' = IF Mutant = "recover_without_sync" THEN bookCands ELSE {bookVis}
          /\ ledVis' = Recovered(ledVis, bookVis)
          /\ ledDur' = Recovered(ledDur, bookVis)
       \/ /\ lock = FREE
          /\ DirSyncCanFail
          /\ UNCHANGED <<bookCands, ledVis, ledDur>>
       \/ /\ lock # FREE
          /\ UNCHANGED <<bookCands, ledVis, ledDur>>
    /\ Goto(p, "idle")
    /\ UNCHANGED <<bookVis, gen, synced, quiet, bakVis, bakDur, created, order, lock,
                   stamp, open, slot, mem, newv, revert, myGen, recheckGen, countVars, flagVars>>

CloseBook(p) ==
    /\ pc[p] = "idle"
    /\ Goto(p, "closed")
    /\ UNCHANGED <<fsVars, lock, stamp, open, slot, mem, newv, revert, myGen, recheckGen, countVars, flagVars>>

StartSave(p) ==
    /\ pc[p] = "idle"
    /\ commits < MaxCommits
    /\ commits' = commits + 1
    /\ revert' = [revert EXCEPT ![p] = FALSE]
    /\ Goto(p, "c_lock")
    /\ UNCHANGED <<fsVars, lock, stamp, open, slot, mem, newv, myGen, recheckGen,
                   nextVer, crashes, cuts, externals, flagVars>>

StartRevert(p) ==
    /\ pc[p] = "idle"
    /\ commits < MaxCommits
    /\ commits' = commits + 1
    /\ revert' = [revert EXCEPT ![p] = TRUE]
    /\ Goto(p, "c_lock")
    /\ UNCHANGED <<fsVars, lock, stamp, open, slot, mem, newv, myGen, recheckGen,
                   nextVer, crashes, cuts, externals, flagVars>>

TakeLock(p) ==
    /\ pc[p] = "c_lock"
    /\ \/ /\ lock = FREE
          /\ lock' = p
          /\ Goto(p, "c_check")
       \/ /\ lock # FREE
          /\ Goto(p, "idle")
          /\ UNCHANGED lock
    /\ UNCHANGED <<fsVars, stamp, open, slot, mem, newv, revert, myGen, recheckGen, countVars, flagVars>>

CheckUnchanged(p) ==
    /\ pc[p] = "c_check"
    /\ IF StampSame(p)
       THEN /\ Goto(p, IF revert[p] THEN "r_slot" ELSE "c_ensure")
            /\ UNCHANGED <<stamp, lock>>
       ELSE IF slot[p] # NONE /\ mem[p].head = bookVis
       THEN /\ stamp' = [stamp EXCEPT ![p] = gen]
            /\ Goto(p, IF revert[p] THEN "r_slot" ELSE "c_ensure")
            /\ UNCHANGED lock
       ELSE /\ Abort(p)
            /\ UNCHANGED stamp
    /\ IF ~StampSame(p) /\ slot[p] # NONE /\ mem[p].head = bookVis /\ Mutant # "keep_stale_slot"
       THEN /\ slot' = [slot EXCEPT ![p] = NONE]
            /\ mem' = [mem EXCEPT ![p] = NoLed]
       ELSE UNCHANGED <<slot, mem>>
    /\ UNCHANGED <<fsVars, open, newv, revert, myGen, recheckGen, countVars, flagVars>>

FoundLedger(p, found) ==
    Led(open[p], IF Mutant = "find_valid_clears_pending" THEN NONE ELSE ledVis[found].pending)

ShowBackupStatus(p) ==
    /\ pc[p] = "idle"
    /\ slot[p] = NONE
    /\ StampSame(p)
    /\ FindValid(open[p]) # NONE
    /\ slot' = [slot EXCEPT ![p] = FindValid(open[p])]
    /\ mem' = [mem EXCEPT ![p] = FoundLedger(p, FindValid(open[p]))]
    /\ UNCHANGED <<fsVars, lock, pc, stamp, open, newv, revert, myGen, recheckGen, countVars, flagVars>>

RevertSlot(p) ==
    /\ pc[p] = "r_slot"
    /\ LET found == IF slot[p] # NONE THEN slot[p] ELSE FindValid(open[p])
           ledger == IF slot[p] # NONE THEN mem[p] ELSE FoundLedger(p, found)
       IN IF found = NONE \/ ledger.head = found
          THEN /\ Abort(p)
               /\ UNCHANGED <<slot, mem>>
          ELSE /\ slot' = [slot EXCEPT ![p] = found]
               /\ mem' = [mem EXCEPT ![p] = ledger]
               /\ Goto(p, "c_ensure")
               /\ UNCHANGED lock
    /\ UNCHANGED <<fsVars, stamp, open, newv, revert, myGen, recheckGen, countVars, flagVars>>

RecoverThenFind(p) ==
    LET recovered == Recovered(ledVis, bookVis)
        cur == open[p]
        usable(b) == recovered[b] # NoLed /\ b \in bakVis
                     /\ (recovered[b].head = cur \/ recovered[b].pending = cur)
        found == IF \E b \in Vers : usable(b)
                 THEN CHOOSE b \in Vers : usable(b) /\ \A c \in Vers : usable(c) => created[c] <= created[b]
                 ELSE NONE
    IN /\ bookCands' = IF Mutant = "recover_without_sync" THEN bookCands ELSE {bookVis}
       /\ ledVis' = recovered
       /\ ledDur' = Recovered(ledDur, bookVis)
       /\ IF found # NONE
          THEN /\ slot' = [slot EXCEPT ![p] = found]
               /\ mem' = [mem EXCEPT ![p] = Led(cur, NONE)]
               /\ Goto(p, "c_temp")
               /\ UNCHANGED lock
          ELSE IF bookVis = cur
          THEN /\ slot' = [slot EXCEPT ![p] = cur]
               /\ mem' = [mem EXCEPT ![p] = Led(cur, NONE)]
               /\ Goto(p, IF Mutant = "ledger_before_backup" THEN "c_create_ledger" ELSE "c_create_backup")
               /\ UNCHANGED lock
          ELSE /\ Abort(p)
               /\ UNCHANGED <<slot, mem>>

EnsureSlot(p) ==
    /\ pc[p] = "c_ensure"
    /\ IF slot[p] # NONE /\ (mem[p].pending = NONE \/ Mutant = "keep_pending_slot")
       THEN /\ Goto(p, "c_temp")
            /\ UNCHANGED <<bookCands, ledVis, ledDur, slot, mem, lock>>
       ELSE \/ RecoverThenFind(p)
            \/ /\ DirSyncCanFail
               /\ Mutant # "recover_without_sync"
               /\ Abort(p)
               /\ UNCHANGED <<bookCands, ledVis, ledDur, slot, mem>>
    /\ UNCHANGED <<bookVis, gen, synced, quiet, bakVis, bakDur, created, order,
                   stamp, open, newv, revert, myGen, recheckGen, countVars, flagVars>>

CreateBackup(p) ==
    /\ pc[p] = "c_create_backup"
    /\ bakVis' = bakVis \cup {slot[p]}
    /\ bakDur' = bakDur \cup {slot[p]}
    /\ Goto(p, IF Mutant = "ledger_before_backup" THEN "c_temp" ELSE "c_create_ledger")
    /\ UNCHANGED <<bookVis, bookCands, gen, synced, quiet, ledVis, ledDur, created, order, lock,
                   stamp, open, slot, mem, newv, revert, myGen, recheckGen, countVars, flagVars>>

CreateLedger(p) ==
    /\ pc[p] = "c_create_ledger"
    /\ StoreLedger(slot[p], Led(slot[p], NONE))
    /\ order' = order + 1
    /\ created' = [created EXCEPT ![slot[p]] = order + 1]
    /\ Goto(p, IF Mutant = "ledger_before_backup" THEN "c_create_backup" ELSE "c_temp")
    /\ UNCHANGED <<bookVis, bookCands, gen, synced, quiet, bakVis, bakDur, lock,
                   stamp, open, slot, mem, newv, revert, myGen, recheckGen, countVars, flagVars>>

WriteTemp(p) ==
    /\ pc[p] = "c_temp"
    /\ newv' = [newv EXCEPT ![p] = IF revert[p] THEN slot[p] ELSE nextVer]
    /\ nextVer' = IF revert[p] THEN nextVer ELSE nextVer + 1
    /\ Goto(p, "c_temp_sync")
    /\ UNCHANGED <<fsVars, lock, stamp, open, slot, mem, revert, myGen, recheckGen,
                   commits, crashes, cuts, externals, flagVars>>

SyncTemp(p) ==
    /\ pc[p] = "c_temp_sync"
    /\ synced' = IF Mutant = "no_temp_sync" THEN synced ELSE synced \cup {newv[p]}
    /\ Goto(p, CASE Mutant = "no_pending" -> "c_recheck"
                 [] Mutant = "settle_before_rename" -> "c_settle"
                 [] OTHER -> "c_pending")
    /\ UNCHANGED <<bookVis, bookCands, gen, quiet, ledVis, ledDur, bakVis, bakDur, created, order, lock,
                   stamp, open, slot, mem, newv, revert, myGen, recheckGen, countVars, flagVars>>

SetPending(p) ==
    /\ pc[p] = "c_pending"
    /\ mem' = [mem EXCEPT ![p] = Led(mem[p].head, newv[p])]
    /\ StoreLedger(slot[p], Led(mem[p].head, newv[p]))
    /\ Goto(p, "c_recheck")
    /\ UNCHANGED <<bookVis, bookCands, gen, synced, quiet, bakVis, bakDur, created, order, lock,
                   stamp, open, slot, newv, revert, myGen, recheckGen, countVars, flagVars>>

RecheckStamp(p) ==
    /\ pc[p] = "c_recheck"
    /\ recheckGen' = [recheckGen EXCEPT ![p] = gen]
    /\ IF (StampSame(p) /\ (bookVis = open[p] \/ Mutant = "stamp_only_recheck")) \/ Mutant = "no_recheck"
       THEN /\ Goto(p, "c_rename")
            /\ UNCHANGED <<ledVis, ledDur, mem, lock>>
       ELSE /\ mem' = [mem EXCEPT ![p] = Led(mem[p].head, NONE)]
            /\ StoreLedger(slot[p], Led(mem[p].head, NONE))
            /\ Abort(p)
    /\ UNCHANGED <<bookVis, bookCands, gen, synced, quiet, bakVis, bakDur, created, order,
                   stamp, open, slot, newv, revert, myGen, countVars, flagVars>>

RenameOverBook(p) ==
    /\ pc[p] = "c_rename"
    /\ bookVis' = newv[p]
    /\ gen' = gen + 1
    /\ myGen' = [myGen EXCEPT ![p] = gen + 1]
    /\ clobbered' = (clobbered \/ gen # stamp[p])
    /\ missed' = (missed \/ (gen # stamp[p] /\ recheckGen[p] # stamp[p]))
    /\ bookCands' = bookCands \cup {newv[p]}
    /\ Goto(p, "c_dirsync")
    /\ UNCHANGED <<synced, quiet, ledVis, ledDur, bakVis, bakDur, created, order, lock,
                   stamp, open, slot, mem, newv, revert, recheckGen, countVars>>

SyncBookDir(p) ==
    /\ pc[p] = "c_dirsync"
    /\ \/ /\ bookCands' = {bookVis}
          /\ Goto(p, IF Mutant = "settle_before_rename" THEN "c_done" ELSE "c_settle")
       \/ /\ DirSyncCanFail
          /\ UNCHANGED bookCands
          /\ Goto(p, IF Mutant = "settle_despite_failed_flush" THEN "c_settle" ELSE "c_done")
    /\ UNCHANGED <<bookVis, gen, synced, quiet, ledVis, ledDur, bakVis, bakDur, created, order, lock,
                   stamp, open, slot, mem, newv, revert, myGen, recheckGen, countVars, flagVars>>

Settle(p) ==
    /\ pc[p] = "c_settle"
    /\ mem' = [mem EXCEPT ![p] = Led(newv[p], NONE)]
    /\ StoreLedger(slot[p], Led(newv[p], NONE))
    /\ Goto(p, IF Mutant = "settle_before_rename" THEN "c_recheck" ELSE "c_done")
    /\ UNCHANGED <<bookVis, bookCands, gen, synced, quiet, bakVis, bakDur, created, order, lock,
                   stamp, open, slot, newv, revert, myGen, recheckGen, countVars, flagVars>>

Finish(p) ==
    /\ pc[p] = "c_done"
    /\ stamp' = [stamp EXCEPT ![p] = myGen[p]]
    /\ open' = [open EXCEPT ![p] = newv[p]]
    /\ Abort(p)
    /\ UNCHANGED <<fsVars, slot, mem, newv, revert, myGen, recheckGen, countVars, flagVars>>

Crash(p) ==
    /\ pc[p] \notin {"closed"}
    /\ crashes < MaxCrashes
    /\ crashes' = crashes + 1
    /\ Goto(p, "closed")
    /\ lock' = IF lock = p THEN FREE ELSE lock
    /\ UNCHANGED <<fsVars, stamp, open, slot, mem, newv, revert, myGen, recheckGen,
                   nextVer, commits, cuts, externals, flagVars>>

PowerCut ==
    /\ cuts < MaxPowerCuts
    /\ cuts' = cuts + 1
    /\ \E kept \in bookCands :
          /\ bookVis' = IF kept \in synced THEN kept ELSE TORN
          /\ bookCands' = {IF kept \in synced THEN kept ELSE TORN}
    /\ gen' = gen + 1
    /\ ledVis' = ledDur
    /\ bakVis' = bakDur
    /\ lock' = FREE
    /\ pc' = [p \in Procs |-> "closed"]
    /\ UNCHANGED <<synced, quiet, ledDur, bakDur, created, order,
                   stamp, open, slot, mem, newv, revert, myGen, recheckGen,
                   nextVer, commits, crashes, externals, flagVars>>

ExternalWrite ==
    /\ externals < MaxExternal
    /\ bookVis # TORN
    /\ externals' = externals + 1
    /\ bookVis' = nextVer
    /\ bookCands' = {nextVer}
    /\ synced' = synced \cup {nextVer}
    /\ nextVer' = nextVer + 1
    /\ gen' = gen + 1
    /\ \/ quiet' = quiet
       \/ quiet' = quiet \cup {gen + 1}
    /\ UNCHANGED <<ledVis, ledDur, bakVis, bakDur, created, order, lock, procVars,
                   commits, crashes, cuts, flagVars>>

Next ==
    \/ \E p \in Procs :
        \/ OpenBook(p) \/ OpenRecover(p) \/ CloseBook(p)
        \/ StartSave(p) \/ StartRevert(p) \/ TakeLock(p) \/ CheckUnchanged(p)
        \/ ShowBackupStatus(p) \/ RevertSlot(p) \/ EnsureSlot(p) \/ CreateBackup(p) \/ CreateLedger(p)
        \/ WriteTemp(p) \/ SyncTemp(p) \/ SetPending(p) \/ RecheckStamp(p)
        \/ RenameOverBook(p) \/ SyncBookDir(p) \/ Settle(p) \/ Finish(p) \/ Crash(p)
    \/ PowerCut
    \/ ExternalWrite

Spec == Init /\ [][Next]_vars

BookNeverTorn == bookVis # TORN

BackupBeforeLedger == \A b \in Vers : ledVis[b] # NoLed => b \in bakVis

RevertReachable ==
    (lock = FREE /\ externals = 0 /\ \E b \in Vers : ledVis[b] # NoLed)
        => FindValid(bookVis) = Original

NoMissedExternalWrite == ~missed

NoExternalWriteOverwritten == ~clobbered
====
