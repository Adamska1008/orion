import { useEffect, useReducer } from 'react';
import { Api, type Detail, type Page, type Task } from '../../lib/api';
import { errorText } from '../../lib/desktop';

const emptyPage: Page = { revision: 0, total: 0, offset: 0, entries: [] };
interface State {
  taskId: string; parent: number; offset: number; pageSize: number; selected: number | null;
  page: Page; directory: Detail | null; detail: Detail | null; loadingPage: boolean;
}
type Action =
  | { type: 'task'; id: string }
  | { type: 'navigate'; parent: number }
  | { type: 'page'; number: number }
  | { type: 'size'; size: number }
  | { type: 'select'; id: number | null }
  | { type: 'loading' }
  | { type: 'loaded'; page: Page; directory: Detail }
  | { type: 'failed' }
  | { type: 'detail'; detail: Detail };

const initial = (taskId: string, pageSize = 15): State => ({ taskId, parent: 0, offset: 0, pageSize, selected: null, page: emptyPage, directory: null, detail: null, loadingPage: false });

function reducer(state: State, action: Action): State {
  switch (action.type) {
    case 'task': return initial(action.id, state.pageSize);
    case 'navigate': return { ...state, parent: action.parent, offset: 0, selected: null, detail: null,
      ...(state.parent !== action.parent || state.offset !== 0 ? { page: emptyPage, directory: null } : {}) };
    case 'page': {
      const offset = (action.number - 1) * state.pageSize;
      if (!Number.isInteger(action.number) || action.number < 1 || offset >= state.page.total || offset === state.offset) return state;
      return { ...state, offset, selected: null, detail: null, page: { ...state.page, offset, entries: [] } };
    }
    case 'size':
      if (action.size === state.pageSize || ![15, 50, 100].includes(action.size)) return state;
      return { ...state, pageSize: action.size, offset: 0, selected: null, detail: null, page: { ...state.page, offset: 0, entries: [] } };
    case 'select': return state.selected === action.id ? state : { ...state, selected: action.id, detail: null };
    case 'loading': return { ...state, loadingPage: true };
    case 'loaded':
      if (state.offset > 0 && state.offset >= action.page.total) return { ...state, offset: 0, selected: null, detail: null, page: { ...emptyPage, total: action.page.total } };
      return { ...state, page: action.page, directory: action.directory, loadingPage: false };
    case 'failed': return { ...state, loadingPage: false };
    case 'detail': return { ...state, detail: action.detail };
  }
}

export function useExplorer(api: Api | null, task: Task, online: boolean, onError: (message: string) => void) {
  const [state, dispatch] = useReducer(reducer, task.id, initial);
  // Reset before children render or effects issue queries for the next task.
  if (state.taskId !== task.id) dispatch({ type: 'task', id: task.id });
  const { parent, offset, pageSize, selected } = state;

  useEffect(() => {
    if (!api || !online || state.taskId !== task.id) return;
    const controller = new AbortController();
    dispatch({ type: 'loading' });
    Promise.all([api.page(task.id, parent, offset, pageSize, controller.signal), api.detail(task.id, parent, controller.signal)])
      .then(([page, directory]) => { if (!controller.signal.aborted) dispatch({ type: 'loaded', page, directory }); })
      .catch(cause => {
        if (!controller.signal.aborted) { onError(errorText(cause)); dispatch({ type: 'failed' }); }
      });
    return () => controller.abort();
  }, [api, task.id, task.revision, state.taskId, parent, offset, pageSize, online, onError]);

  useEffect(() => {
    if (!api || !online || selected === null || state.taskId !== task.id) return;
    const controller = new AbortController();
    api.detail(task.id, selected, controller.signal)
      .then(detail => { if (!controller.signal.aborted) dispatch({ type: 'detail', detail }); })
      .catch(cause => { if (!controller.signal.aborted) onError(errorText(cause)); });
    return () => controller.abort();
  }, [api, task.id, task.revision, state.taskId, selected, online, onError]);

  return { ...state,
    pageCount: Math.max(1, Math.ceil(state.page.total / pageSize)),
    pageNumber: Math.floor(offset / pageSize) + 1,
    navigate: (parent: number) => dispatch({ type: 'navigate', parent }),
    changePage: (number: number) => { if (online) dispatch({ type: 'page', number }); },
    changePageSize: (size: number) => dispatch({ type: 'size', size }),
    setSelected: (id: number | null) => dispatch({ type: 'select', id }),
  };
}
