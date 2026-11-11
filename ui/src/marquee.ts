export const detailsGutter = 10;
export const tileInset = 2;

export type ItemRectangle = {
  firstRow: number;
  lastRow: number;
  firstColumn: number;
  lastColumn: number;
  columns: number;
};

export function hitsItem(area: ItemRectangle | null, index: number) {
  if (!area) return false;
  const row = Math.floor(index / area.columns);
  const col = index % area.columns;
  return (
    row >= area.firstRow &&
    row <= area.lastRow &&
    col >= area.firstColumn &&
    col <= area.lastColumn
  );
}

// Sorted half-open intervals keep full rows compact and preserve positions on reflow.
export function rectangleSpans(area: ItemRectangle | null, total: number) {
  if (!area) return [];
  const { firstRow, lastRow, firstColumn, lastColumn, columns } = area;
  if (firstColumn === 0 && lastColumn === columns - 1)
    return [firstRow * columns, Math.min(total, (lastRow + 1) * columns)];
  const spans: number[] = [];
  for (let row = firstRow; row <= lastRow; row++) {
    const start = row * columns + firstColumn;
    if (start < total)
      spans.push(start, Math.min(total, row * columns + lastColumn + 1));
  }
  return spans;
}

export function toggleSpans(a: number[], b: number[]) {
  const spans: number[] = [];
  let i = 0,
    j = 0;
  while (i < a.length && j < b.length) {
    if (a[i] === b[j]) {
      i++;
      j++;
    } else spans.push(a[i] < b[j] ? a[i++] : b[j++]);
  }
  for (; i < a.length; i++) spans.push(a[i]);
  for (; j < b.length; j++) spans.push(b[j]);
  return spans;
}

export function inSpans(spans: number[], index: number) {
  let lo = 0,
    hi = spans.length;
  while (lo < hi) {
    const mid = (lo + hi) >>> 1;
    if (spans[mid] <= index) lo = mid + 1;
    else hi = mid;
  }
  return lo % 2 === 1;
}
