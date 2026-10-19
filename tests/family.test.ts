import { describe, expect, test } from 'bun:test';
import { familyFor, familyColor, normalizeModelName } from '../src/lib/format';

describe('model families', () => {
  test('minimax models resolve to Minimax family', () => {
    expect(familyFor('MiniMax-M3.1-Flash')).toBe('Minimax');
    expect(familyFor('minimax-m3.1-flash')).toBe('Minimax');
    expect(familyFor('minimax/minimax-m3.1-flash')).toBe('Minimax');
    expect(familyFor('minimax-m3')).toBe('Minimax');
    expect(familyFor('minimax-text-01')).toBe('Minimax');
    expect(familyFor('abab6.5s')).toBe('Minimax');
  });

  test('unknown models fall back to Other', () => {
    expect(familyFor('totally-unknown-model')).toBe('Other');
    expect(familyFor('')).toBe('Other');
  });

  test('minimax brand color is defined', () => {
    expect(familyColor('Minimax', 0)).toBe('#FF3763');
    expect(familyColor('MiniMax', 0)).toBe('#FF3763');
  });

  test('normalizeModelName handles decorations and provider prefixes', () => {
    expect(normalizeModelName('minimax/MiniMax-M3.1-Flash[ffe]')).toBe('minimax-m3.1-flash');
    expect(normalizeModelName('MiniMax-M3.1-Flash')).toBe('minimax-m3.1-flash');
  });
});
