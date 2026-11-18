// 书内搜索定位：先确保命中属于当前小节，再从该小节的渲染文本中定位。
// 本工具不解析 Markdown AST，不改原文，也不把图片目录拼进原文偏移。
export function clearSearchMarks(root: HTMLElement | null): void {
  root?.querySelectorAll("mark.native-search").forEach((mark) => {
    const parent = mark.parentNode;
    if (!parent) return;
    parent.replaceChild(document.createTextNode(mark.textContent ?? ""), mark);
    parent.normalize();
  });
}

/**
 * query 是后端从原文截取的精确片段；titleSearch=true 时只查小节标题。
 * .md block 由 ReadingView 按 contentBlocks 的顺序排列；图片不占 DOM 文本位置。
 */
export function findSearchRange(root: HTMLElement, query: string, unitId: number, titleSearch = false): Range | null {
  if (!query) return null;
  const scope = root.querySelector<HTMLElement>(`[data-search-unit="${unitId}"]`) ?? root;
  const targets = titleSearch
    ? [scope]
    : Array.from(scope.querySelectorAll<HTMLElement>("[data-unit-content]"));
  for (const target of targets.length ? targets : [scope]) {
    const range = findInElement(target, query);
    if (range) return range;
  }
  return null;
}

function findInElement(root: HTMLElement, query: string): Range | null {
  const values = Array.from(root.textContent ?? "");
  const needle = Array.from(query);
  let matchStart = -1;
  outer: for (let start = 0; start + needle.length <= values.length; start += 1) {
    for (let index = 0; index < needle.length; index += 1) {
      if (values[start + index].toLocaleLowerCase() !== needle[index].toLocaleLowerCase()) continue outer;
    }
    matchStart = start;
    break;
  }
  if (matchStart < 0) return null;
  // 必须限制在单个 Text 节点：Tags/strong/code 断开时回退块块开头，不用跨节点强制包裹。
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
  const nodes: Text[] = [];
  let node = walker.nextNode();
  while (node) {
    nodes.push(node as Text);
    node = walker.nextNode();
  }
  // 先处理最常用的同一文本节点命中，不引入跨多个标签的选区。
  for (const text of nodes) {
    const haystack = Array.from(text.data);
    for (let start = 0; start + needle.length <= haystack.length; start += 1) {
      if (haystack.slice(start, start + needle.length).join("").toLocaleLowerCase() === query.toLocaleLowerCase()) {
        const range = document.createRange();
        const before = haystack.slice(0, start).join("").length;
        range.setStart(text, before);
        range.setEnd(text, before + query.length);
        return range;
      }
    }
  }
  // 被粗体等打断的一段：允许首尾文本节点包裹，中间完整节点都是当前小块内部。
  let offset = 0;
  let first: { node: Text; offset: number } | null = null;
  let last: { node: Text; offset: number } | null = null;
  const end = matchStart + needle.length;
  for (const text of nodes) {
    const next = offset + Array.from(text.data).length;
    if (!first && matchStart < next) first = { node: text, offset: Array.from(text.data).slice(0, matchStart - offset).join("").length };
    if (last === null && end <= next) last = { node: text, offset: Array.from(text.data).slice(0, end - offset).join("").length };
    if (first && last) break;
    offset = next;
  }
  if (first && last) {
    const range = document.createRange();
    range.setStart(first.node, first.offset);
    range.setEnd(last.node, last.offset);
    return range;
  }
  return null;
}
