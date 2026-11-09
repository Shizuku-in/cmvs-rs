# CPZ 读写

`cmvs_core::cpz::ArchiveIndex` 统一读取和替换写回 CPZ3、CPZ5（模式 5～9）、CPZ6（模式 1）、CPZ7（模式 0/1）。CPZ1/2 目前仅识别签名。

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
