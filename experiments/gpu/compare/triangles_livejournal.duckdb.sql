.bail on
SET threads=24;
CREATE TABLE raw_edges AS SELECT * FROM read_csv('/data/dataset/graph/soc-LiveJournal/edge.facts',
    delim='\t', header=false, auto_detect=false, comment='#',
    columns={'src':'INTEGER','dst':'INTEGER'});
CREATE TABLE E AS SELECT DISTINCT src, dst FROM raw_edges;
SELECT 'stats' AS kind, (SELECT count(*) FROM raw_edges) AS input_rows,
    count(*) AS edge_count, (SELECT count(*) FROM raw_edges)-count(*) AS duplicates,
    least(min(src),min(dst)) AS node_min,
    greatest(max(src),max(dst)) AS node_max FROM E;
DROP TABLE raw_edges;
WITH indegrees AS (SELECT dst AS b, count(*)::HUGEINT AS n FROM E GROUP BY dst),
     outdegrees AS (SELECT src AS b, count(*)::HUGEINT AS n FROM E GROUP BY src)
SELECT 'wedges' AS kind, coalesce(sum(i.n*o.n),0)::HUGEINT AS wedges
FROM indegrees i JOIN outdegrees o USING(b);
COPY E TO '/tmp/triangles-reference-data/livejournal.set.tsv' (FORMAT CSV, HEADER false, DELIMITER '\t');
.timer on
SELECT count(*) AS triangle_count FROM E ab JOIN E bc ON ab.dst = bc.src JOIN E ac ON ac.src = ab.src AND ac.dst = bc.dst;
SELECT count(*) AS triangle_count FROM E ab JOIN E bc ON ab.dst = bc.src JOIN E ac ON ac.src = ab.src AND ac.dst = bc.dst;
SELECT count(*) AS triangle_count FROM E ab JOIN E bc ON ab.dst = bc.src JOIN E ac ON ac.src = ab.src AND ac.dst = bc.dst;
SELECT count(*) AS triangle_count FROM E ab JOIN E bc ON ab.dst = bc.src JOIN E ac ON ac.src = ab.src AND ac.dst = bc.dst;
SELECT count(*) AS triangle_count FROM E ab JOIN E bc ON ab.dst = bc.src JOIN E ac ON ac.src = ab.src AND ac.dst = bc.dst;
SELECT count(*) AS triangle_count FROM E ab JOIN E bc ON ab.dst = bc.src JOIN E ac ON ac.src = ab.src AND ac.dst = bc.dst;
.timer off
