/** Safe arithmetic evaluator — recursive descent, no eval (see main.ts). */

export function evaluate(input: string): number {
  if (/\d\s+\d/.test(input)) {
    throw new Error('ambiguous: digits separated by spaces');
  }
  const src = input.replace(/\s+/g, '');
  let pos = 0;

  const peek = () => src[pos];
  const eat = (ch: string) => {
    if (src[pos] === ch) {
      pos++;
      return true;
    }
    return false;
  };

  function factor(): number {
    if (eat('-')) return -factor();
    if (eat('(')) {
      const v = expr();
      if (!eat(')')) throw new Error('missing )');
      return v;
    }
    const start = pos;
    while (pos < src.length && /[\d.]/.test(src[pos])) pos++;
    if (start === pos) throw new Error(`unexpected “${peek() ?? 'end'}”`);
    const value = Number(src.slice(start, pos));
    if (Number.isNaN(value)) throw new Error(`bad number “${src.slice(start, pos)}”`);
    return value;
  }

  function term(): number {
    let v = factor();
    for (;;) {
      if (eat('*')) v *= factor();
      else if (eat('/')) {
        const d = factor();
        if (d === 0) throw new Error('division by zero');
        v /= d;
      } else if (eat('%')) v %= factor();
      else return v;
    }
  }

  function expr(): number {
    let v = term();
    for (;;) {
      if (eat('+')) v += term();
      else if (eat('-')) v -= term();
      else return v;
    }
  }

  const result = expr();
  if (pos !== src.length) throw new Error(`unexpected “${src[pos]}”`);
  if (!Number.isFinite(result)) throw new Error('result is not finite');
  return result;
}
