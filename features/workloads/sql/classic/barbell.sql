-- Two triangles joined at their first vertices by the bridge relation.
SELECT DISTINCT r.c0, r.c1, s.c1, u.c0, u.c1, v.c1
FROM "mini-linq/raw/0" AS r
JOIN "mini-linq/raw/1" AS s ON s.c0 = r.c1
JOIN "mini-linq/raw/2" AS t ON t.c0 = r.c0 AND t.c1 = s.c1
JOIN "mini-linq/raw/3" AS bridge ON bridge.c0 = r.c0
JOIN "mini-linq/raw/4" AS u ON u.c0 = bridge.c1
JOIN "mini-linq/raw/5" AS v ON v.c0 = u.c1
JOIN "mini-linq/raw/6" AS w ON w.c0 = u.c0 AND w.c1 = v.c1
ORDER BY 1, 2, 3, 4, 5, 6;
