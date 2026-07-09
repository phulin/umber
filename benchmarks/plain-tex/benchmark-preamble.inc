% Shared deterministic setup for all benchmark engines.
\font\benchmarkrm=cmr10
\font\benchmarkmi=cmmi10
\font\benchmarksy=cmsy10
\font\benchmarkex=cmex10
\font\benchmarksevenrm=cmr10 at 7pt
\font\benchmarkfiverm=cmr10 at 5pt
\font\benchmarksevenmi=cmmi10 at 7pt
\font\benchmarkfivemi=cmmi10 at 5pt
\font\benchmarksevensy=cmsy10 at 7pt
\font\benchmarkfivesy=cmsy10 at 5pt
\font\benchmarksevenex=cmex10 at 7pt
\font\benchmarkfiveex=cmex10 at 5pt
\let\benchmarkit=\benchmarkmi
\let\benchmarkbf=\benchmarksy
\benchmarkrm

\textfont0=\benchmarkrm
\scriptfont0=\benchmarksevenrm
\scriptscriptfont0=\benchmarkfiverm
\textfont1=\benchmarkmi
\scriptfont1=\benchmarksevenmi
\scriptscriptfont1=\benchmarkfivemi
\textfont2=\benchmarksy
\scriptfont2=\benchmarksevensy
\scriptscriptfont2=\benchmarkfivesy
\textfont3=\benchmarkex
\scriptfont3=\benchmarksevenex
\scriptscriptfont3=\benchmarkfiveex
\skewchar\benchmarkmi="7F
\skewchar\benchmarksevenmi="7F
\skewchar\benchmarkfivemi="7F
\skewchar\benchmarksy="30
\skewchar\benchmarksevensy="30
\skewchar\benchmarkfivesy="30

\mathcode`a="7161 \mathcode`b="7162 \mathcode`c="7163
\mathcode`d="7164 \mathcode`e="7165 \mathcode`f="7166
\mathcode`g="7167 \mathcode`h="7168 \mathcode`i="7169
\mathcode`j="716A \mathcode`k="716B \mathcode`l="716C
\mathcode`m="716D \mathcode`n="716E \mathcode`o="716F
\mathcode`p="7170 \mathcode`q="7171 \mathcode`r="7172
\mathcode`s="7173 \mathcode`t="7174 \mathcode`u="7175
\mathcode`v="7176 \mathcode`w="7177 \mathcode`x="7178
\mathcode`y="7179 \mathcode`z="717A
\mathcode`+="202B \mathcode`-="2200 \mathcode`=="303D
\mathcode`(="4028 \mathcode`)="5029 \mathcode`,="602C
\mathcode`|="026A
\delcode`(="028300 \delcode`)="029301 \delcode`|="26A30C
\mathchardef\BenchmarkSum="1350
\mathchardef\BenchmarkProd="1351
\mathchardef\BenchmarkArrow="3221
\mathchardef\BenchmarkInfinity="0231
\mathchardef\BenchmarkAlpha="710B
\mathchardef\BenchmarkBeta="710C
\mathchardef\BenchmarkGamma="710D
\def\BenchmarkRadical#1{\radical"270370 {#1}}

\hsize=420pt
\vsize=700pt
\topskip=10pt
\maxdepth=4pt
\baselineskip=12pt
\lineskip=1pt
\lineskiplimit=0pt
\parindent=0pt
\parskip=0pt
\parfillskip=0pt plus 1fil
\overfullrule=0pt
\hbadness=10000
\vbadness=10000
\binoppenalty=700
\relpenalty=500
\mathsurround=0pt
\scriptspace=.5pt
\thinmuskip=3mu
\medmuskip=4mu plus 2mu minus 4mu
\thickmuskip=5mu plus 5mu
\delimiterfactor=901
\delimitershortfall=5pt
\nulldelimiterspace=1.2pt
\output={\shipout\box255}
\def\benchmarkbye{\par\vfill\penalty-10000 \end}
\def\benchmarkeject{\par\penalty-10000 }
\def\benchmarknobreak{\penalty10000 }
\def\benchmarkoffinterlineskip{%
  \baselineskip=-1000pt \lineskip=0pt \lineskiplimit=1000pt}
\def\-{\discretionary{-}{}{}}
\hyphenchar\benchmarkrm=-1
