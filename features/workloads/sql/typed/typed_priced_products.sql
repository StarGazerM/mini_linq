SELECT DISTINCT band.label, p.productkey, p.name, p.released
FROM Price AS band JOIN Product AS p USING (price);
