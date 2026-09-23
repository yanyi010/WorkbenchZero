import { describe, expect, it } from 'vitest';
import { evaluate } from './calc';

describe('quickcalc evaluator', () => {
  it('evaluates basic arithmetic with correct precedence', () => {
    expect(evaluate('1 + 2 * 3')).toBe(7);
    expect(evaluate('(1 + 2) * 3')).toBe(9);
    expect(evaluate('10 / 4')).toBe(2.5);
    expect(evaluate('7 % 3')).toBe(1);
  });

  it('handles unary minus and nesting', () => {
    expect(evaluate('-5 + 3')).toBe(-2);
    expect(evaluate('-(2 + 3) * 4')).toBe(-20);
    expect(evaluate('2 * -(3 + 4)')).toBe(-14);
  });

  it('ignores whitespace', () => {
    expect(evaluate('  1   +\n2 ')).toBe(3);
  });

  it('supports decimals', () => {
    expect(evaluate('0.5 * 4')).toBe(2);
    expect(evaluate('1.5 + 1.5')).toBe(3);
  });

  it('rejects malformed input instead of throwing opaque errors', () => {
    expect(() => evaluate('1 +')).toThrow();
    expect(() => evaluate('(1 + 2')).toThrow('missing )');
    expect(() => evaluate('foo + 1')).toThrow();
    expect(() => evaluate('1 2')).toThrow();
  });

  it('rejects division by zero and non-finite results', () => {
    expect(() => evaluate('1 / 0')).toThrow('division by zero');
    expect(() => evaluate('0 / 0')).toThrow('division by zero');
  });
});
