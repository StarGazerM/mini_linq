-- Multiplication binds more tightly than addition.
SELECT DISTINCT c0, c1 FROM "mini-linq/raw/0"
WHERE c0 + 2 * c1 >= 3 ORDER BY 1, 2;
