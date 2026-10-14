# 独立词法排名消融：BM25 与字符／词 ngram

此检查点只添加隔离的合成评测器和 CI，没有替换生产分词、排名、词项版本或配置。没有下载模型、调用推理或读取用户资料。源文档、问题和相关性标注均为本评测原创；没有复制万象词库或输入法语言模型。

## 固定试验

264 条合成文档，其中 262 条在本次范围内且未撤销。51 个查询分为 41 个预设有答案来源的查询与 10 个预设无答案查询。包含中文自然问题、稀有词、英文词序、编号／版本／路径、长短来源、否定／后续纠正、错字、同义改写及范围负例。

- A：实际调用生产 `event_lexical.terms` 与 SQLite `candidates`，按 distinct 命中数排序。
- B：保持相同分词，改为 BM25，`k1=1.2, b=.75`。
- B0：仅把 B 的 `b` 改为 0，作为长度归一化消融，未做更多参数搜索。
- C：完整 ASCII + CJK 2+3gram，BM25。
- D：完整 ASCII + CJK 1+2gram，作为单字噪声对照，BM25。
- E：在 C 基础上增加相邻英文词 bigram，只能与 C 对比归因词序收益。未安装中文分词器，未把手写词典或逐字切分冒充中文词 ngram。

所有策略完整保留 ASCII token 的大小写、数字及连接符，不把编号、版本和路径切为字符片段。英文相邻词按词值与单空格生成词项键，证据仍保存原始空格／换行及其 UTF-8 span/hash。

BM25 的 tf 保留重复词项，query 去重，df、平均长度和文档数来自当前完整可见合成集合，不能从 top100 候选估计。复现公式为 `log(1+(N-df+.5)/(df+.5)) * tf*(k1+1)/(tf+k1*(1-b+b*dl/avgdl))`。这是独立 Python 实现，不声称与 Lucene 的 norms 压缩或评分缩放逐位相同。

两组查询预算分别为：实际 32 个 distinct 词项上限；仅放开此上限的诊断。二者仍保留 8,192 postings、1,024 候选块、100 个返回来源上限。超限不截短问题。候选遍历按词项轮询；对已经进入候选窗口的 BM25 文档使用完整 query 词项评分。报告记录两组的受限率、截断、Top1/Top5、Recall@5、MRR@10、nDCG@5、词项规模和每题前十项。

## 本次合成结果

以生产 32 词上限计算；Top1/Top5 的分母是 41 个正例，最后一列分母是 10 个无答案查询。

| 策略 | Top1 命中 | Top5 命中 | MRR@10 | 词项超限查询 | 无答案但有候选 |
|---|---:|---:|---:|---:|---:|
| A：distinct count | 33/41 | 39/41 | .8646 | 0/51 | 2/10 |
| B：同分词 BM25 | 31/41 | 40/41 | .8463 | 0/51 | 2/10 |
| B0：同分词 BM25，b=0 | 34/41 | 40/41 | .8878 | 0/51 | 2/10 |
| C：2+3gram BM25 | 30/41 | 39/41 | .8220 | 1/51 | 2/10 |
| D：1+2gram BM25 | 30/41 | 39/41 | .8159 | 2/51 | 4/10 |
| E：C + 英文词 bigram | 32/41 | 39/41 | .8463 | 1/51 | 2/10 |

“无答案但有候选”不等于生成了错误答案，也不是完整相关性 precision。检索可以找到提及同一对象但未包含答案的来源，本试验没有运行回答模型。否定文本与旧方案可作为相关证据，但不能据此认定当前事实；否定／纠正用例不能替代事实采用评测。

可见语料的 unique postings：A/B/B0 为 6,675，C 为 11,979，D 为 13,914，E 为 12,035。2+3gram 在此集合增加约 79% postings，却没有在既有 query 预算内得到净收益。放开 query 词项上限后 C 的 Top5 为 40/41；这是预算诊断，不能当作生产可用结果。1+2gram 增加噪声及受限问题，不能因某个改写例偶然靠单字碰撞命中就称为语义理解。

稀有词组的 8 个正例中，A Top1 为 5/8、Top5 为 7/8，B 为 7/8、8/8。另一方面，3 个要求具体结论的长来源问题在 B 中从第一位降到第三位。它们的原文为 3,483–3,485 字节、915 个词项；24–26 字节、4 个词项的短消息先返回，30 字节、8 个词项且缺少目标编号的其他来源排第二。B0 去掉长度归一化后这三个长来源恢复第一位。这只说明本合成构造存在长度权衡，不据此选择生产默认 b。

6 个纯编号/路径/版本查询全部保持完整边界；额外 12 个带精确标识的正例诊断中，A/B/B0/C/E 的 Top1 都包含全部标识，D 为 9/12。“token 不被切碎”不保证混合查询永远按该编号优先。

## 已验证与未验证

已执行：12 项方法测试，包括手算 BM25、重复 tf/query 去重、df 的范围隔离、b=0 消融、query 预算、候选窗口/轮询、编号近邻与大小写、词 bigram 空白归一化、原文 hash 失配和无答案指标定义。两个独立进程分别用 `PYTHONHASHSEED=1/2` 重跑，fixture 与完整结果逐字节一致。

严格边界：

- 这是单词法通道的已知合成资料试验，问题、标注和评测由同一实现者编写，未经盲测；不是用户真实语料指标。同构的长短文和词序问题也不是独立样本，不作统计显著性或泛化改善声明。
- 每个合成文档当作一个 chunk，包括长来源。未经过真实分块器，长文降权结论不能直接外推生产最佳 chunk 聚合。
- 没有语义 top100、RRF、Rust 精确锚点优先层或最终 8KiB 响应预算；不能把表格叫作最终 RecallCard 质量。
- scope、撤销及证据测试是隔离集合的过滤/原文 hash 合同，不是生产 Vault、Rust 宿主或权限竞态安全验收。
- CJK trigram 与词 shingle 当前不能直接通过 Rust 的既有词项证据验证；接入需词项版本、manifest、存储统计和宿主核验一同升级。
- 未测大库延迟、磁盘索引大小、中文词分词质量、真实模型及真实用户会话。结果不支持直接替换生产默认策略。

## 复现

仅需要 Python 3.10+ 和仓库源码，无额外依赖。输出必须使用新的评测目录，不要指定真实资料库。

```sh
python3 -m unittest discover -s scripts/lexical_ablation -p 'test_*.py' -v
PYTHONHASHSEED=1 python3 scripts/lexical_ablation/evaluate.py --repo "$PWD" --out /tmp/recallcard-lexical-ablation
```

输出 `fixture.json` 与 `results.json` 包含固定合成来源、预设标注、所有指标、结果范围以及 fixture／脚本／生产词法源码 SHA256。独立工作流 `lexical-ablation.yml` 执行方法测试及跨哈希随机种子复现，并保存七天合成产物。工作流运行成功仅代表这项试验可复现。

## 一手方法来源

- [Lucene BM25Similarity](https://lucene.apache.org/core/10_3_1/core/org/apache/lucene/search/similarities/BM25Similarity.html)：IDF、tf/长度参数。
- [Lucene CJKBigramFilter](https://lucene.apache.org/core/10_3_1/analysis/common/org/apache/lucene/analysis/cjk/CJKBigramFilter.html)：CJK 双字组、单字回退与可选 unigram。
- [Elasticsearch shingle token filter](https://www.elastic.co/docs/reference/text-analysis/analysis-shingle-tokenfilter)：相邻词序列，区别于字符切片。

没有从上述来源复制实现或语料；只按公开公式与定义实现小型消融。
