COPY kkutu_en (_id, type, mean, hit, theme, flag) FROM stdin;
apple	n	x	0	0	0
\.
COPY kkutu_ko (_id, type, mean, hit, flag, theme) FROM stdin;
기차	1,1	x	0	0	0
리본	1	x	0	1	0
늠름하다	6	x	0	0	0
방언말	1	x	0	8	0
어인정	1	x	0	2	0
껌	1	x	0	0	0
CD롬	1	x	0	1	0
사과	0,1	x	0	0	0
\.
COPY kkutu_manner_en (_id) FROM stdin;
