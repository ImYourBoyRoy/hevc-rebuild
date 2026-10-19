/** Build the existing token insertion without owning DOM focus or persisted templates. */
export function insertRenameTemplateToken(current: string, token: string, selectionStart: number | null, selectionEnd: number | null): { value: string; caret: number } {
  const start = selectionStart ?? current.length;
  const end = selectionEnd ?? current.length;
  return { value: `${current.slice(0, start)}${token}${current.slice(end)}`, caret: start + token.length };
}
