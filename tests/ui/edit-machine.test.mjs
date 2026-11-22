import test from 'node:test';
import assert from 'node:assert/strict';
import { EVENTS, READ, THEN, asking, editing, hasUnsavedText, lookOf, saving, step } from '../../app/ui/edit-machine.js';

const TARGET = { ix: 7, fragment: null };
const RUN = () => {};
const FAILURE = { kind: 'saveFailed', message: 'no room' };

const STATES = {
  read: READ,
  editClean: editing(),
  editDirty: editing(true),
  editFailed: editing(true, FAILURE),
  savingStay: saving(THEN.stay),
  savingRead: saving(THEN.read),
  savingGo: saving(THEN.go(TARGET)),
  savingClose: saving(THEN.close),
  savingReload: saving(THEN.reload),
  savingApply: saving(THEN.apply(RUN)),
  askingGo: asking(THEN.go(TARGET)),
  askingClose: asking(THEN.close),
  askingReload: asking(THEN.reload),
  askingApply: asking(THEN.apply(RUN)),
};

const EVENT_OF = {
  enter: { type: 'enter', editable: true },
  esc: { type: 'esc' },
  saveKey: { type: 'saveKey' },
  dirtyChanged: { type: 'dirtyChanged', dirty: true },
  saved: { type: 'saved' },
  failed: { type: 'failed', failure: FAILURE },
  go: { type: 'go', target: TARGET },
  close: { type: 'close' },
  reload: { type: 'reload' },
  apply: { type: 'apply', run: RUN },
  dialogSave: { type: 'dialogSave' },
  dialogDiscard: { type: 'dialogDiscard' },
  dialogKeep: { type: 'dialogKeep' },
};

const SAME = 'same';
const fx = (...names) => names;

const EXPECTED = {
  read: {
    enter: [editing(), fx('beginEdit')],
    go: [SAME, fx('navigate')],
    close: [SAME, fx('closeWindow')],
    reload: [SAME, fx('reload')],
    apply: [SAME, fx('applyWhole')],
  },
  editClean: {
    esc: [READ, fx('endEdit')],
    dirtyChanged: [editing(true), fx()],
    go: [READ, fx('endEdit', 'navigate')],
    close: [READ, fx('endEdit', 'closeWindow')],
    reload: [READ, fx('endEdit', 'reload')],
    apply: [SAME, fx('applyWhole')],
  },
  editDirty: {
    esc: [saving(THEN.read), fx('lookRead', 'save')],
    saveKey: [saving(THEN.stay), fx('save')],
    dirtyChanged: [editing(true), fx()],
    go: [asking(THEN.go(TARGET)), fx('askUnsaved')],
    close: [asking(THEN.close), fx('askUnsaved')],
    reload: [asking(THEN.reload), fx('askUnsaved')],
    apply: [asking(THEN.apply(RUN)), fx('askUnsaved')],
  },
  editFailed: {
    esc: [saving(THEN.read), fx('lookRead', 'save')],
    saveKey: [saving(THEN.stay), fx('save')],
    dirtyChanged: [editing(true, FAILURE), fx()],
    go: [asking(THEN.go(TARGET)), fx('askUnsaved')],
    close: [asking(THEN.close), fx('askUnsaved')],
    reload: [asking(THEN.reload), fx('askUnsaved')],
    apply: [asking(THEN.apply(RUN)), fx('askUnsaved')],
  },
  savingStay: {
    esc: [saving(THEN.read), fx('lookRead')],
    saved: [editing(), fx('settle')],
    failed: [editing(true, FAILURE), fx('showFailure', 'lookEdit')],
    go: [saving(THEN.go(TARGET)), fx('lookRead')],
    close: [saving(THEN.close), fx('lookRead')],
    apply: [saving(THEN.apply(RUN)), fx('lookRead')],
  },
  savingRead: {
    saved: [READ, fx('settle', 'endEdit')],
    failed: [editing(true, FAILURE), fx('showFailure', 'lookEdit')],
    go: [saving(THEN.go(TARGET)), fx()],
    close: [saving(THEN.close), fx()],
    apply: [saving(THEN.apply(RUN)), fx()],
  },
  savingGo: {
    saved: [READ, fx('settle', 'endEdit', 'navigate')],
    failed: [editing(true, FAILURE), fx('showFailure', 'lookEdit')],
    go: [saving(THEN.go(TARGET)), fx()],
    close: [saving(THEN.close), fx()],
    apply: [saving(THEN.apply(RUN)), fx()],
  },
  savingClose: {
    saved: [READ, fx('settle', 'endEdit', 'closeWindow')],
    failed: [editing(true, FAILURE), fx('showFailure', 'lookEdit')],
    close: [saving(THEN.close), fx()],
  },
  savingReload: {
    saved: [READ, fx('settle', 'endEdit', 'reload')],
    failed: [editing(true, FAILURE), fx('showFailure', 'lookEdit')],
    go: [saving(THEN.go(TARGET)), fx()],
    close: [saving(THEN.close), fx()],
    apply: [saving(THEN.apply(RUN)), fx()],
  },
  savingApply: {
    saved: [READ, fx('settle', 'endEdit', 'applyWhole')],
    failed: [editing(true, FAILURE), fx('showFailure', 'lookEdit')],
    go: [saving(THEN.go(TARGET)), fx()],
    close: [saving(THEN.close), fx()],
    apply: [saving(THEN.apply(RUN)), fx()],
  },
  askingApply: {
    dialogSave: [saving(THEN.apply(RUN)), fx('hideDialog', 'lookRead', 'save')],
    dialogDiscard: [READ, fx('hideDialog', 'endEdit', 'restoreChapter', 'applyWhole')],
    dialogKeep: [editing(true), fx('hideDialog', 'refocus')],
  },
  askingGo: {
    dialogSave: [saving(THEN.go(TARGET)), fx('hideDialog', 'lookRead', 'save')],
    dialogDiscard: [READ, fx('hideDialog', 'endEdit', 'navigate')],
    dialogKeep: [editing(true), fx('hideDialog', 'refocus')],
  },
  askingClose: {
    dialogSave: [saving(THEN.close), fx('hideDialog', 'lookRead', 'save')],
    dialogDiscard: [READ, fx('hideDialog', 'endEdit', 'closeWindow')],
    dialogKeep: [editing(true), fx('hideDialog', 'refocus')],
  },
  askingReload: {
    dialogSave: [saving(THEN.reload), fx('hideDialog', 'lookRead', 'save')],
    dialogDiscard: [READ, fx('hideDialog', 'endEdit', 'reload')],
    dialogKeep: [editing(true), fx('hideDialog', 'refocus')],
  },
};

const allPairs = Object.entries(STATES).flatMap(([stateName, state]) => EVENTS.map((eventName) => ({ stateName, state, eventName })));

test('every event in the machine has a sample and every sample is an event', () => {
  assert.deepEqual(Object.keys(EVENT_OF).sort(), [...EVENTS].sort());
  assert.equal(allPairs.length, 14 * 13);
});

for (const { stateName, state, eventName } of allPairs) {
  test(`${stateName} on ${eventName}`, () => {
    const result = step(state, EVENT_OF[eventName]);
    const expected = EXPECTED[stateName][eventName] ?? [SAME, fx()];
    assert.deepEqual(result.state, expected[0] === SAME ? state : expected[0]);
    assert.deepEqual(result.effects.map((effect) => effect.fx), expected[1]);
  });
}

test('the expected table names only real states and events', () => {
  for (const [stateName, row] of Object.entries(EXPECTED)) {
    assert.ok(stateName in STATES, stateName);
    for (const eventName of Object.keys(row)) assert.ok(EVENTS.includes(eventName), eventName);
  }
});

test('a page with no text refuses Edit and stays in Read', () => {
  const result = step(READ, { type: 'enter', editable: false });
  assert.deepEqual(result.state, READ);
  assert.deepEqual(result.effects, [{ fx: 'refuseEdit' }]);
});

test('Esc on a dirty chapter never asks and a clean one never saves', () => {
  assert.ok(!step(editing(true), EVENT_OF.esc).effects.some((effect) => effect.fx === 'askUnsaved'));
  assert.ok(!step(editing(), EVENT_OF.esc).effects.some((effect) => effect.fx === 'save'));
  assert.ok(!step(editing(), EVENT_OF.saveKey).effects.some((effect) => effect.fx === 'save'));
});

test('dirty text only reaches Read through a save or a discard', () => {
  for (const { stateName, state, eventName } of allPairs) {
    const { state: next } = step(state, EVENT_OF[eventName]);
    if (!hasUnsavedText(state) || next.mode !== 'read') continue;
    assert.ok(['saved', 'dialogDiscard'].includes(eventName), `${stateName} on ${eventName}`);
  }
});

test('no save can start while one is running', () => {
  for (const [name, state] of Object.entries(STATES)) {
    if (state.mode !== 'saving') continue;
    for (const eventName of EVENTS) {
      const { effects } = step(state, EVENT_OF[eventName]);
      assert.ok(!effects.some((effect) => effect.fx === 'save'), `${name} on ${eventName}`);
    }
  }
});

test('every failure lands in Edit, marked dirty, with its message', () => {
  for (const [name, state] of Object.entries(STATES)) {
    if (state.mode !== 'saving') continue;
    const { state: next, effects } = step(state, EVENT_OF.failed);
    assert.deepEqual(next, editing(true, FAILURE), name);
    assert.deepEqual(effects[0], { fx: 'showFailure', failure: FAILURE }, name);
  }
});

test('a close during a save survives a later chapter change', () => {
  const { state } = step(saving(THEN.close), EVENT_OF.go);
  assert.deepEqual(state, saving(THEN.close));
});

test('the look follows the state', () => {
  const looks = Object.fromEntries(Object.entries(STATES).map(([name, state]) => [name, lookOf(state)]));
  assert.deepEqual(looks, {
    read: 'read',
    editClean: 'edit',
    editDirty: 'edit',
    editFailed: 'edit',
    savingStay: 'edit',
    savingRead: 'read',
    savingApply: 'read',
    savingGo: 'read',
    savingClose: 'read',
    savingReload: 'read',
    askingApply: 'dialog',
    askingGo: 'dialog',
    askingClose: 'dialog',
    askingReload: 'dialog',
  });
});

test('unsaved text is reported until the file holds it', () => {
  const unsaved = Object.entries(STATES).filter(([, state]) => hasUnsavedText(state)).map(([name]) => name);
  assert.deepEqual(unsaved, [
    'editDirty', 'editFailed', 'savingStay', 'savingRead', 'savingGo', 'savingClose', 'savingReload', 'savingApply',
    'askingGo', 'askingClose', 'askingReload', 'askingApply',
  ]);
});

test('walking a typo from E to Read takes one save', () => {
  let state = READ;
  const effects = [];
  for (const event of [EVENT_OF.enter, EVENT_OF.dirtyChanged, EVENT_OF.esc, EVENT_OF.esc, EVENT_OF.saved]) {
    const result = step(state, event);
    state = result.state;
    effects.push(...result.effects.map((effect) => effect.fx));
  }
  assert.deepEqual(state, READ);
  assert.equal(effects.filter((name) => name === 'save').length, 1);
});

test('apply runs at once from Read and from a clean Edit, which it does not leave', () => {
  const fromRead = step(READ, EVENT_OF.apply);
  assert.equal(fromRead.state, READ);
  assert.deepEqual(fromRead.effects, [{ fx: 'applyWhole', run: RUN }]);
  const clean = editing();
  const fromEdit = step(clean, EVENT_OF.apply);
  assert.equal(fromEdit.state, clean);
  assert.deepEqual(fromEdit.effects, [{ fx: 'applyWhole', run: RUN }]);
});

test('apply over unsaved text asks first and each answer behaves', () => {
  const asked = step(editing(true), EVENT_OF.apply);
  assert.deepEqual(asked.state, asking(THEN.apply(RUN)));
  assert.deepEqual(asked.effects, [{ fx: 'askUnsaved' }]);
  const kept = step(asked.state, EVENT_OF.dialogKeep);
  assert.ok(!kept.effects.some((effect) => effect.fx === 'applyWhole'));
  const discarded = step(asked.state, EVENT_OF.dialogDiscard);
  assert.equal(discarded.state, READ);
  assert.deepEqual(discarded.effects.at(-1), { fx: 'applyWhole', run: RUN });
  const saving = step(asked.state, EVENT_OF.dialogSave);
  assert.equal(saving.state.mode, 'saving');
  assert.ok(!saving.effects.some((effect) => effect.fx === 'applyWhole'));
  const saved = step(saving.state, EVENT_OF.saved);
  assert.equal(saved.state, READ);
  assert.deepEqual(saved.effects.at(-1), { fx: 'applyWhole', run: RUN });
  const failed = step(saving.state, EVENT_OF.failed);
  assert.ok(!failed.effects.some((effect) => effect.fx === 'applyWhole'));
});
