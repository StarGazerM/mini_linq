-- A sale must have all four dimension witnesses.
SELECT DISTINCT sale.c0, sale.c2, sale.c3
FROM "mini-linq/raw/0" AS sale
JOIN "mini-linq/raw/1" AS sold_date ON sold_date.c0 = sale.c1
JOIN "mini-linq/raw/2" AS item ON item.c0 = sale.c2
JOIN "mini-linq/raw/3" AS store ON store.c0 = sale.c3
JOIN "mini-linq/raw/4" AS demographic ON demographic.c0 = sale.c4
ORDER BY 1, 2, 3;
