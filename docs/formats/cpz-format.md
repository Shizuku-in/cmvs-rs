# CPZ 读写

`cmvs_core::cpz::ArchiveIndex` 统一读取和替换写回 CPZ3、CPZ5（模式 5～9）、CPZ6（模式 1）、CPZ7（模式 0/1）。另外只读支持 CVNS 时代的 CPZ1/CPZ2（见文末），不支持写回。

## 替换写回

```rust
use cmvs_core::cpz::ArchiveIndex;
use std::{collections::BTreeMap, fs::File};

let mut source = File::open("input.cpz")?;
let index = ArchiveIndex::read(&mut source)?;
let replacements = BTreeMap::from([
    (index.find(b"main.ps3")?, std::fs::read("main.ps3")?),
]);
let mut output = File::options().write(true).create_new(true).open("output.cpz")?;
index.write_replacements(&mut source, &mut output, &replacements)?;
```

需要显式参数时，先用 `read_with_cpz5_mode`、`read_with_cpz7_parameters` 或 `script_control::archive_index_from_start` 读取索引；写回自动沿用选定参数。条目按编号替换，可区分重复文件名。

写回保留版本、条目数量与顺序、原始名称、目录、密钥、打包/加密模式及索引保留字段；CPZ3 的索引填充和 CPZ7 的索引密钥块也保留。所有源资源先验证密文校验和，未替换资源原样复制密文，替换资源重新加密，最后更新索引摘要和头部校验和。CPZ3/5 使用 32 位数据偏移，CPZ6/7 使用 64 位偏移。

源流须匹配所读索引，输出流必须为空。输入不修改；失败可能留下不完整输出。单项资源及索引沿用 core 的分配上限，不将整个封包装入内存。CPZ6 读取仍不强制验证头部/索引摘要，写回会重新计算这两项。

## 新建封包

`Cpz3Index::write(&mut output, &entries)` 接受按顺序排列的 `(Vec<u8>, Vec<u8>)` 文件名与内容，默认使用零密钥。名称保留原始编码字节，长度为 1～4096，不含 NUL；允许同名条目。CPZ5/6 使用 `ArchiveDirectory`、`ArchiveFile` 显式描述目录和明文内容：

```rust
use cmvs_core::cpz::{ArchiveDirectory, ArchiveFile, Cpz5Index, Cpz5Mode, Cpz6Index};

let directories = vec![ArchiveDirectory {
    name: b"code".to_vec(),
    files: vec![ArchiveFile { name: b"main.ps3".to_vec(), data: script_bytes }],
}];
Cpz5Index::write(&mut output, Cpz5Mode::Mode9, &directories)?;
// CPZ6 使用另一个新的空输出流。
Cpz6Index::write(&mut another_output, &directories)?;
```

CPZ5 新建可选模式 5～9；CPZ6 使用模式 1。新建 CPZ5/6 默认启用资源加密，使用确定性的头部及目录/文件密钥。目录名称可为空，文件名称规则与 CPZ3 相同，同名条目保留顺序；允许空目录和空封包。空 CPZ5 不携带模式信息，读取时可显式指定所需模式。CPZ7 目前提供已有封包的替换写回。

## 验证

```sh
cargo test --offline --locked --test cpz_write --test cpz3 --test cpz5 --test cpz --test cpz7
```

独立合成向量覆盖各版本和模式、每包参数、重复名称、目录与索引填充、空/变长资源及加密尾字节；检查替换内容回读、未替换密文不变；新建 CPZ3/5/6 覆盖目录、名称、空资源和加密边界，并拒绝错误条目、损坏或不匹配的输入及非空输出。9 个原版脚本包共 1,113 项已完成替换回读，覆盖全部上述版本和 CPZ5 模式；另以 7 个原版脚本包的 907 项内容新建 CPZ5/6，全部回读一致。CPZ6 头部摘要和校验和公式另与 25 个原版封包核对。

## CPZ1 / CPZ2（CVNS，只读）

两者都是 CPZ3 沿用的平坦索引：每条记录依次为记录长度、资源大小、数据偏移（低 32 位）、偏移高位、校验和、密钥、以 NUL 结尾的文件名，之后可以有填充。名称保留原始大小写，查找时两侧都折叠 ASCII 大小写（Shift-JIS 双字节原样比较）；原引擎另有无扩展名的前缀查找，对应 `lookup_prefix`。

- **CPZ1**（2004 年两作）：16 字节明文头部（签名、文件数、索引长度、保留）。索引与每个资源各自从密钥下标 0 开始，按字节做 `(b ^ key[i & 63]) - 108`，密钥是 exe 内 64 字节 Shift-JIS 短语逐字节加 10。没有校验和，`read_entry_verified` 等同 `read_entry`。
- **CPZ2**（2005、2009 年三作）：20 字节头部，文件数、索引长度分别异或 `0xe47c59f3`、`0x3f71de2a`，+12 是索引密文校验和（与 CPZ3 同一算法和种子），+16 的索引密钥异或 `0x77777777 ^ 0x37a9f45b`。按 32 位字解密：`rotr((w ^ (table[n & 15] + key)) - 0x15c3e7, shift)`，尾部不足 4 字节的部分为 `(b ^ ((table + key) >> 4i)) + 55`；`shift` 由密钥的 8 个半字节与 5 异或后取低 4 位再加 8。`table` 是同一短语（第 47 字节不同）与从 55 起每步减 41 的字节流异或后的 16 个小端字。每条记录的密钥异或 `0x796c3afd`。

常量与流程取自 2005 年 `cvns.exe`（读取器 `0x4247c0`/`0x424be0`，解密 `0x424730`/`0x424a60`，校验和 `0x424a20`）。5 个游戏目录共 37 个封包的全部条目可读出，CPZ2 的 23 个封包逐条通过校验和；按扩展名核对了 `PB2A`、`MSK0`、`MV2X`、`OggS` 签名。合成向量由测试内独立编写的加密器生成（`tests/cpz1_cpz2.rs`）。
