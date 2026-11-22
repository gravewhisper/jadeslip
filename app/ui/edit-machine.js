export const READ = { mode: 'read' };

export const editing = (dirty = false, failure = null) => ({ mode: 'edit', dirty, failure });
export const saving = (then) => ({ mode: 'saving', then });
export const asking = (then) => ({ mode: 'asking', then });

export const THEN = {
  stay: { to: 'stay' },
  read: { to: 'read' },
  close: { to: 'close' },
  reload: { to: 'reload' },
  go: (target) => ({ to: 'go', target }),
  apply: (run) => ({ to: 'apply', run }),
};

function afterLeaving(then) {
  if (then.to === 'go') return [{ fx: 'endEdit' }, { fx: 'navigate', target: then.target }];
  if (then.to === 'close') return [{ fx: 'endEdit' }, { fx: 'closeWindow' }];
  if (then.to === 'reload') return [{ fx: 'endEdit' }, { fx: 'reload' }];
  if (then.to === 'apply') return [{ fx: 'endEdit' }, { fx: 'applyWhole', run: then.run }];
  return [{ fx: 'endEdit' }];
}

function leaving(state, then) {
  if (!state.dirty) return { state: READ, effects: afterLeaving(then) };
  return { state: asking(then), effects: [{ fx: 'askUnsaved' }] };
}

function startSave(then) {
  const look = then.to === 'stay' ? [] : [{ fx: 'lookRead' }];
  return { state: saving(then), effects: [...look, { fx: 'save' }] };
}

function retarget(state, then) {
  if (state.then.to === 'close') return { state, effects: [] };
  return { state: saving(then), effects: state.then.to === 'stay' ? [{ fx: 'lookRead' }] : [] };
}

const TRANSITIONS = {
  read: {
    enter: (state, event) => (event.editable
      ? { state: editing(), effects: [{ fx: 'beginEdit' }] }
      : { state, effects: [{ fx: 'refuseEdit' }] }),
    go: (state, event) => ({ state, effects: [{ fx: 'navigate', target: event.target }] }),
    close: (state) => ({ state, effects: [{ fx: 'closeWindow' }] }),
    reload: (state) => ({ state, effects: [{ fx: 'reload' }] }),
    apply: (state, event) => ({ state, effects: [{ fx: 'applyWhole', run: event.run }] }),
  },
  edit: {
    dirtyChanged: (state, event) => ({ state: editing(event.dirty, state.failure), effects: [] }),
    esc: (state) => (state.dirty ? startSave(THEN.read) : { state: READ, effects: [{ fx: 'endEdit' }] }),
    saveKey: (state) => (state.dirty ? startSave(THEN.stay) : { state, effects: [] }),
    go: (state, event) => leaving(state, THEN.go(event.target)),
    close: (state) => leaving(state, THEN.close),
    reload: (state) => leaving(state, THEN.reload),
    apply: (state, event) => (state.dirty
      ? leaving(state, THEN.apply(event.run))
      : { state, effects: [{ fx: 'applyWhole', run: event.run }] }),
  },
  saving: {
    saved: (state) => (state.then.to === 'stay'
      ? { state: editing(), effects: [{ fx: 'settle' }] }
      : { state: READ, effects: [{ fx: 'settle' }, ...afterLeaving(state.then)] }),
    failed: (_state, event) => ({
      state: editing(true, event.failure),
      effects: [{ fx: 'showFailure', failure: event.failure }, { fx: 'lookEdit' }],
    }),
    esc: (state) => (state.then.to === 'stay' ? retarget(state, THEN.read) : { state, effects: [] }),
    go: (state, event) => retarget(state, THEN.go(event.target)),
    apply: (state, event) => retarget(state, THEN.apply(event.run)),
    close: (state) => ({
      state: saving(THEN.close),
      effects: state.then.to === 'stay' ? [{ fx: 'lookRead' }] : [],
    }),
  },
  asking: {
    dialogSave: (state) => {
      const started = startSave(state.then);
      return { state: started.state, effects: [{ fx: 'hideDialog' }, ...started.effects] };
    },
    dialogDiscard: (state) => ({
      state: READ,
      effects: [
        { fx: 'hideDialog' },
        ...(state.then.to === 'apply'
          ? [{ fx: 'endEdit' }, { fx: 'restoreChapter' }, { fx: 'applyWhole', run: state.then.run }]
          : afterLeaving(state.then)),
      ],
    }),
    dialogKeep: () => ({ state: editing(true), effects: [{ fx: 'hideDialog' }, { fx: 'refocus' }] }),
  },
};

export const EVENTS = [
  'enter', 'esc', 'saveKey', 'dirtyChanged', 'saved', 'failed',
  'go', 'close', 'reload', 'apply', 'dialogSave', 'dialogDiscard', 'dialogKeep',
];

export function step(state, event) {
  const transition = TRANSITIONS[state.mode][event.type];
  return transition ? transition(state, event) : { state, effects: [] };
}

export function lookOf(state) {
  if (state.mode === 'edit') return 'edit';
  if (state.mode === 'asking') return 'dialog';
  if (state.mode === 'saving') return state.then.to === 'stay' ? 'edit' : 'read';
  return 'read';
}

export const hasUnsavedText = (state) => state.mode === 'saving' || state.mode === 'asking'
  || (state.mode === 'edit' && state.dirty);
