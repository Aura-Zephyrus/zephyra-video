//! 解码图像缓冲（DPB）与参考帧列表。
//!
//! 对应 Chromium `media/gpu/h264_dpb.{h,cc}`（`H264DPB`）。
//!
//! # 为什么需要它
//!
//! DXVA2 的 `ID3D11VideoDecoder` **不做**参考帧管理：驱动只认
//! `DXVA_PicParams_H264.RefFrameList` 里给出的那几个索引，它不知道
//! 「哪一帧是上一帧」。把参考帧的来龙去脉（哪些帧还在用、哪些该被
//! 淘汰、参考列表怎么排序）算出来，是**喂给硬件**之前的必要一步。
//!
//! 纯逻辑：没有 unsafe，因此在任何机器上都能单测。
//!
//! # 与 Chromium 的差别
//!
//! Chromium 的 DXVA2 路径里，这部分工作由**软件** H.264 解码器
//! （`media/gpu/h264_decoder.cc`）做，硬件只负责像素。本 crate 没有
//! 软件解码器，于是这半边搬到了这里 —— 字段与规则照抄规格与
//! Chromium，两边可以对着看。

use crate::h264::parser::{DecRefPicMarking, H264ParseError, H264SliceHeader, H264Sps};

/// DPB 里的最大图片数。
///
/// 规格允许 `max_num_ref_frames` 到 16（frame_mbs_only_flag == 1）或
/// 32（场编码）。DXVA2 的 `RefFrameList` 只有 16 项，所以**帧编码**下
/// 我们最多同时持有 16 张参考帧 —— 这也是 `MAX_PICTURES` 取 16 的原因。
pub const MAX_PICTURES: usize = 16;

/// 一张已解码图像的状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecodedPicture {
    /// DXVA 的图像索引（7 位）。DPB 内的图片索引互不相同。
    pub picture_index: u8,
    /// `frame_num`（spec 7.4.3）。
    pub frame_num: u16,
    /// 长期参考帧的索引（`long_term_frame_idx`）；短期帧为 `None`。
    pub long_term_frame_idx: Option<u16>,
    /// `TopFieldOrderCnt`。
    pub top_field_order_cnt: i32,
    /// `BottomFieldOrderCnt`。
    pub bottom_field_order_cnt: i32,
    /// 该帧是否还被用作参考（`unused for reference` 的反面）。
    pub is_reference: bool,
    /// 该帧是「不存在的参考帧」占位（spec 8.2.4.2.1）。
    ///
    /// 参考帧列表修改可能指向一个 DPB 里没有的短期帧 —— 那时规格要求
    /// 放一个占位进去（`non_existing`），驱动会把它当成一张全灰的图。
    pub non_existing: bool,
}

impl DecodedPicture {
    /// 一个「不存在的参考帧」占位（spec 8.2.4.2.1）。
    ///
    /// ⚠️ Chromium 在引用到不存在的图片时**直接失败**，而不是塞占位
    /// （`H264Decoder::ModifyReferencePicList()`）。这个构造器留给想按
    /// 规格那条路走的调用方；本 crate 内部与 Chromium 一致。
    pub fn non_existing(frame_num: u16) -> Self {
        Self {
            picture_index: 0,
            frame_num,
            long_term_frame_idx: None,
            top_field_order_cnt: 0,
            bottom_field_order_cnt: 0,
            is_reference: false,
            non_existing: true,
        }
    }

    /// 该帧是否是长期参考帧。
    pub fn is_long_term(&self) -> bool {
        self.long_term_frame_idx.is_some()
    }

    /// 长期参考帧的 `long_term_pic_num`；短期帧为 0。
    ///
    /// ⚠️ 规格里 `long_term_pic_num`（MMCO 2 用它指认）与
    /// `long_term_frame_idx`（MMCO 3 / 6 用它）是**两个**字段，
    /// 但 Chromium 把它们当成同一个（`h264_decoder.cc` 的
    /// `UpdatePicNums()`：`pic->long_term_pic_num = pic->long_term_frame_idx;`）。
    ///
    /// 这里跟 Chromium 一样 —— DXVA2 的 `FrameNumList` 要的也是这个值，
    /// 两边对不上时，出了问题没法判断是谁的锅。
    pub fn long_term_pic_num(&self) -> u16 {
        self.long_term_frame_idx.unwrap_or(0)
    }
}

/// 一张图像的 POC 计算结果（spec 8.2.1）。
///
/// 对应 Chromium `H264Picture` 的 `pic_order_cnt_msb` / `pic_order_cnt_lsb`
/// / `top_field_order_cnt` / `bottom_field_order_cnt` / `frame_num_offset`。
///
/// 为什么把它做成一个结构体而不是两个 `i32`：这五个值是**一起**算出来的，
/// 而其中三个（msb / lsb / frame_num_offset）要留给**下一张**图像用 ——
/// 拆开返回就会丢掉「它们是同一张图算出来的」这个事实。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PictureOrderCnt {
    /// `TopFieldOrderCnt`。
    pub top_field_order_cnt: i32,
    /// `BottomFieldOrderCnt`。
    pub bottom_field_order_cnt: i32,
    /// `pic_order_cnt_msb`（`pic_order_cnt_type == 0` 时有意义）。
    pub pic_order_cnt_msb: i32,
    /// 原始的 `pic_order_cnt_lsb`（**不是**算出来的 POC）。
    pub pic_order_cnt_lsb: i32,
    /// `frame_num_offset`（`pic_order_cnt_type` 为 1 / 2 时有意义）。
    pub frame_num_offset: i32,
}

/// 解码图像缓冲。
#[derive(Debug, Clone)]
pub struct DecodedPictureBuffer {
    /// DPB 里的图片（短期 + 长期，混在一起）。
    pictures: Vec<DecodedPicture>,
    /// `max_num_ref_frames`（来自 SPS）。
    max_num_ref_frames: u32,
    /// 下一个图片索引起点（见 [`Self::allocate_picture_index`]）。
    next_picture_index: u8,
    /// 上一张**参考**图像的 `pic_order_cnt_msb`
    /// （Chromium `prev_ref_pic_order_cnt_msb_`）。
    prev_ref_pic_order_cnt_msb: i32,
    /// 上一张**参考**图像的原始 `pic_order_cnt_lsb`
    /// （Chromium `prev_ref_pic_order_cnt_lsb_`）。
    prev_ref_pic_order_cnt_lsb: i32,
    /// 上一张**参考**图像的 `TopFieldOrderCnt`
    /// （Chromium `prev_ref_top_field_order_cnt_`）。
    prev_ref_top_field_order_cnt: i32,
    /// 上一张**参考**图像是否执行过 MMCO 5
    /// （Chromium `prev_ref_has_memmgmnt5_`）。
    prev_ref_has_mmco5: bool,
    /// 上一张图像（不论是否参考）的 `frame_num`
    /// （Chromium `prev_frame_num_`）。
    prev_frame_num: i32,
    /// 上一张图像（不论是否参考）的 `frame_num_offset`
    /// （Chromium `prev_frame_num_offset_`）。
    prev_frame_num_offset: i32,
    /// 上一张图像（不论是否参考）是否执行过 MMCO 5
    /// （Chromium `prev_has_memmgmnt5_`）。
    prev_has_mmco5: bool,
}

impl Default for DecodedPictureBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl DecodedPictureBuffer {
    /// 新建一个空的 DPB。
    pub fn new() -> Self {
        Self {
            pictures: Vec::new(),
            max_num_ref_frames: 0,
            next_picture_index: 0,
            prev_ref_pic_order_cnt_msb: -1,
            prev_ref_pic_order_cnt_lsb: -1,
            prev_ref_top_field_order_cnt: -1,
            prev_ref_has_mmco5: false,
            prev_frame_num: 0,
            prev_frame_num_offset: 0,
            prev_has_mmco5: false,
        }
    }

    /// 清空 DPB 与全部状态（对应 `Reset()`）。
    pub fn reset(&mut self) {
        self.pictures.clear();
        self.next_picture_index = 0;
        self.prev_ref_pic_order_cnt_msb = -1;
        self.prev_ref_pic_order_cnt_lsb = -1;
        self.prev_ref_top_field_order_cnt = -1;
        self.prev_ref_has_mmco5 = false;
        self.prev_frame_num = 0;
        self.prev_frame_num_offset = 0;
        self.prev_has_mmco5 = false;
    }

    /// DPB 里的图片（按插入顺序）。
    pub fn pictures(&self) -> &[DecodedPicture] {
        &self.pictures
    }

    /// 分配一个图片索引。
    ///
    /// 索引必须在 DPB 内**唯一** —— 驱动靠它区分参考帧。做法是：从上次
    /// 分配的位置往后找第一个 DPB 里没在用的索引。
    ///
    /// ⚠️ 为什么要「往后找」而不是「找第一个没用的」：调用方是在**解码
    /// 之前**拿索引的（它要写进 `DXVA_PicParams_H264.CurrPic.Index7Bits`），
    /// 那时这张图还没进 DPB。若每次都从头找，连续两次分配会拿到**同一个**
    /// 索引 —— 驱动会把后一张画到前一张的纹理上，画面静默错位。
    ///
    /// ⚠️ 索引空间只有 [`MAX_PICTURES`] 个（0..16），不是 128 个：
    /// DXVA2 的 `Index7Bits` 虽然有 7 位，但 `RefFrameList` 只有 16 项，
    /// 而且 Chromium 的图片缓冲池也是 16 个（`picture_index` 来自那个池）。
    /// 分配 16 个以上的索引会让驱动认不出「哪个索引对应哪张画面」。
    pub fn allocate_picture_index(&mut self) -> u8 {
        let used: Vec<u8> = self.pictures.iter().map(|p| p.picture_index).collect();
        for offset in 0..MAX_PICTURES as u8 {
            let candidate = (self.next_picture_index + offset) % MAX_PICTURES as u8;
            if !used.contains(&candidate) {
                self.next_picture_index = (candidate + 1) % MAX_PICTURES as u8;
                return candidate;
            }
        }
        // 16 张全占用 ⇒ 规格与 DXVA2 都装不下。这不是「换个索引继续」能
        // 解决的 —— 参考帧太多是**码流**的问题。返回一个必定冲突的索引，
        // 让上层发现「DPB 满了」，而不是悄悄给出错误画面。
        u8::MAX
    }

    /// `MaxFrameNum`（spec 7.4.3）。
    fn max_frame_num(sps: &H264Sps) -> u32 {
        1u32 << (sps.log2_max_frame_num_minus4 + 4)
    }

    /// `MaxPicOrderCntLsb`（spec 7.4.3）。
    fn max_pic_order_cnt_lsb(sps: &H264Sps) -> u32 {
        1u32 << (sps.log2_max_pic_order_cnt_lsb_minus4 + 4)
    }

    /// 计算当前图像的 `TopFieldOrderCnt` / `BottomFieldOrderCnt`。
    ///
    /// 对应 spec 8.2.1（三种 `pic_order_cnt_type`），实现照抄 Chromium
    /// `H264Decoder::CalculatePicOrderCounts()`。
    ///
    /// ⚠️ 这个函数**不**更新 DPB 里的 POC 状态 —— 状态在
    /// [`Self::commit_picture`] 里更新（Chromium 在 `FinishPicture()` 里
    /// 更新）。两者的差别很要紧：**上一张图像的 POC 状态要等它真的解码完
    /// 才该被替换**，提前替换会让解码失败的图污染下一张图的 POC。
    pub fn compute_picture_order_cnt(
        &self,
        sps: &H264Sps,
        slice: &H264SliceHeader,
    ) -> Result<PictureOrderCnt, H264ParseError> {
        let frame_num = slice.frame_num as i32;
        let max_frame_num = Self::max_frame_num(sps) as i32;
        match sps.pic_order_cnt_type {
            0 => {
                // spec 8.2.1.1
                let max_lsb = Self::max_pic_order_cnt_lsb(sps) as i32;
                let lsb = slice.pic_order_cnt_lsb as i32;
                // 「上一张**参考**图像」的 POC 状态 —— 非参考图像不参与。
                let (prev_msb, prev_lsb) = if slice.idr_pic_flag {
                    (0, 0)
                } else if self.prev_ref_has_mmco5 {
                    // MMCO 5 把 POC 归零：MSB 清零，LSB 沿用上一张参考帧的
                    // `TopFieldOrderCnt`。
                    (0, self.prev_ref_top_field_order_cnt)
                } else {
                    (
                        self.prev_ref_pic_order_cnt_msb,
                        self.prev_ref_pic_order_cnt_lsb,
                    )
                };
                // ⚠️ 这里只调整**一次**。把调整写进持久状态、再拿它去算
                // `msb`，等于把同一轮调整做了两遍 —— POC 会整整差一个
                // `MaxPicOrderCntLsb`。
                let mut msb = prev_msb;
                if lsb < prev_lsb && (prev_lsb - lsb) >= max_lsb / 2 {
                    msb += max_lsb;
                } else if lsb > prev_lsb && (lsb - prev_lsb) > max_lsb / 2 {
                    msb -= max_lsb;
                }
                let top = msb + lsb;
                let bottom = top + slice.delta_pic_order_cnt_bottom;
                Ok(PictureOrderCnt {
                    top_field_order_cnt: top,
                    bottom_field_order_cnt: bottom,
                    pic_order_cnt_msb: msb,
                    pic_order_cnt_lsb: lsb,
                    frame_num_offset: self.prev_frame_num_offset,
                })
            }
            1 => {
                // spec 8.2.1.2
                let mut frame_num_offset = if self.prev_has_mmco5 {
                    0
                } else {
                    self.prev_frame_num_offset
                };
                if slice.idr_pic_flag {
                    frame_num_offset = 0;
                } else if self.prev_frame_num > frame_num {
                    frame_num_offset += max_frame_num;
                }
                let cycle_len = sps.num_ref_frames_in_pic_order_cnt_cycle as i32;
                let mut abs_frame_num = 0;
                if cycle_len != 0 {
                    abs_frame_num = frame_num_offset + frame_num;
                }
                if slice.nal_ref_idc == 0 && abs_frame_num > 0 {
                    abs_frame_num -= 1;
                }
                let mut expected = 0;
                if abs_frame_num > 0 {
                    if cycle_len == 0 {
                        return Err(H264ParseError::InvalidStream);
                    }
                    // ⚠️ 是 `(absFrameNum - 1)`，不是 `absFrameNum` ——
                    // 规格里 `frameNumInPicOrderCntCycle` 从 0 开始数。
                    let cycle_cnt = (abs_frame_num - 1) / cycle_len;
                    let frame_num_in_cycle = (abs_frame_num - 1) % cycle_len;
                    expected = cycle_cnt * sps.expected_delta_per_pic_order_cnt_cycle;
                    for index in 0..=frame_num_in_cycle {
                        let offset = sps
                            .offset_for_ref_frame
                            .get(index as usize)
                            .copied()
                            .ok_or(H264ParseError::InvalidStream)?;
                        expected += offset;
                    }
                }
                if slice.nal_ref_idc == 0 {
                    expected += sps.offset_for_non_ref_pic;
                }
                let top = expected + slice.delta_pic_order_cnt0;
                let bottom = top + sps.offset_for_top_to_bottom_field + slice.delta_pic_order_cnt1;
                Ok(PictureOrderCnt {
                    top_field_order_cnt: top,
                    bottom_field_order_cnt: bottom,
                    pic_order_cnt_msb: expected,
                    pic_order_cnt_lsb: slice.pic_order_cnt_lsb as i32,
                    frame_num_offset,
                })
            }
            2 => {
                // spec 8.2.1.3
                let mut frame_num_offset = if self.prev_has_mmco5 {
                    0
                } else {
                    self.prev_frame_num_offset
                };
                if slice.idr_pic_flag {
                    frame_num_offset = 0;
                } else if self.prev_frame_num > frame_num {
                    frame_num_offset += max_frame_num;
                }
                let temp = if slice.idr_pic_flag {
                    0
                } else if slice.nal_ref_idc == 0 {
                    2 * (frame_num_offset + frame_num) - 1
                } else {
                    2 * (frame_num_offset + frame_num)
                };
                Ok(PictureOrderCnt {
                    top_field_order_cnt: temp,
                    bottom_field_order_cnt: temp,
                    pic_order_cnt_msb: temp,
                    pic_order_cnt_lsb: slice.pic_order_cnt_lsb as i32,
                    frame_num_offset,
                })
            }
            _ => Err(H264ParseError::UnsupportedStream),
        }
    }

    /// 解码完一张图像后把它放进 DPB，并按引用帧标记维护 DPB。
    ///
    /// 对应 spec 8.2.5（`dec_ref_pic_marking` 的效果）与 Chromium 的
    /// `H264Decoder::FinishPicture()`。
    ///
    /// `picture_index` 必须由调用方**在解码前**分配好
    /// （[`Self::allocate_picture_index`]）：它要写进
    /// `DXVA_PicParams_H264.CurrPic.Index7Bits`，那是驱动认识当前图像的
    /// 唯一凭据 —— 解码完再分配就晚了。
    ///
    /// `order_cnt` 必须是同一张图
    /// [`Self::compute_picture_order_cnt`] 算出来的结果。
    pub fn commit_picture(
        &mut self,
        sps: &H264Sps,
        slice: &H264SliceHeader,
        picture_index: u8,
        order_cnt: &PictureOrderCnt,
    ) -> DecodedPicture {
        self.max_num_ref_frames = sps.max_num_ref_frames;

        let is_reference = slice.nal_ref_idc != 0;
        let picture = DecodedPicture {
            picture_index,
            frame_num: slice.frame_num as u16,
            long_term_frame_idx: None,
            top_field_order_cnt: order_cnt.top_field_order_cnt,
            bottom_field_order_cnt: order_cnt.bottom_field_order_cnt,
            is_reference,
            non_existing: false,
        };

        let mut had_mmco5 = false;
        if slice.idr_pic_flag {
            // IDR 把 DPB 清空（spec 8.2.5.1）。
            self.pictures.clear();
            self.pictures.push(picture);
        } else if is_reference {
            if slice.adaptive_ref_pic_marking_mode_flag {
                had_mmco5 = self.apply_mmco(slice, &picture);
            } else {
                self.sliding_window(slice);
                self.pictures.push(picture);
            }
        } else {
            // 非参考帧不进 DPB。
            self.pictures.push(picture);
        }
        self.pictures
            .retain(|p| p.is_reference || p.picture_index == picture.picture_index);

        // Chromium `FinishPicture()` 的状态更新：
        // `prev_ref_*` 只有**参考**图像才更新，`prev_frame_num` /
        // `prev_frame_num_offset` / `prev_has_mmco5` 每一张都更新。
        if is_reference {
            self.prev_ref_has_mmco5 = had_mmco5;
            self.prev_ref_top_field_order_cnt = order_cnt.top_field_order_cnt;
            self.prev_ref_pic_order_cnt_msb = order_cnt.pic_order_cnt_msb;
            self.prev_ref_pic_order_cnt_lsb = order_cnt.pic_order_cnt_lsb;
        }
        self.prev_frame_num = slice.frame_num as i32;
        self.prev_frame_num_offset = order_cnt.frame_num_offset;
        self.prev_has_mmco5 = had_mmco5;
        picture
    }

    /// 滑动窗口（spec 8.2.5.3）：DPB 满了就淘汰最老的一张短期参考帧。
    fn sliding_window(&mut self, slice: &H264SliceHeader) {
        let capacity = self.max_num_ref_frames.max(1) as usize;
        let reference_count = self.pictures.iter().filter(|p| p.is_reference).count();
        if reference_count < capacity {
            return;
        }
        // 淘汰「最老」的那张：frame_num 距离当前帧最远的那张。
        let max_frame_num = 1u32 << 16;
        let current = slice.frame_num;
        let oldest = self
            .pictures
            .iter()
            .enumerate()
            .filter(|(_, p)| p.is_reference && !p.is_long_term())
            .max_by_key(|(_, p)| {
                // 距离 = (current - frame_num) mod MaxFrameNum，越大越老。
                (current.wrapping_sub(p.frame_num as u32)) % max_frame_num
            })
            .map(|(index, _)| index);
        if let Some(index) = oldest {
            self.pictures[index].is_reference = false;
        }
    }

    /// 应用 MMCO 指令（spec 8.2.5.4）。
    ///
    /// 返回值：是否执行过 MMCO 5（它会把 POC 归零）。
    fn apply_mmco(&mut self, slice: &H264SliceHeader, picture: &DecodedPicture) -> bool {
        let max_frame_num = 1u32 << 16;
        let current = slice.frame_num;
        let mut had_mmco5 = false;
        for marking in &slice.dec_ref_pic_marking {
            match marking.memory_management_control_operation {
                0 => break,
                1 => {
                    // 把 frame_num = (当前 - diff - 1) 的短期帧标记为不用。
                    let target = (current.wrapping_sub(marking.difference_of_pic_nums_minus1 + 1))
                        % max_frame_num;
                    for p in self.pictures.iter_mut() {
                        if p.is_reference && !p.is_long_term() && p.frame_num as u32 == target {
                            p.is_reference = false;
                        }
                    }
                }
                2 => {
                    // 把 long_term_pic_num 对应的长期帧标记为不用。
                    for p in self.pictures.iter_mut() {
                        if p.is_reference && p.is_long_term() {
                            if let Some(idx) = p.long_term_frame_idx {
                                if idx as u32 == marking.long_term_pic_num {
                                    p.is_reference = false;
                                }
                            }
                        }
                    }
                }
                3 | 6 => {
                    // 当前帧变成长期帧（op 3 还会先淘汰同索引的旧帧）。
                    let idx = marking.long_term_frame_idx;
                    for p in self.pictures.iter_mut() {
                        if p.is_reference && p.is_long_term() {
                            if let Some(existing) = p.long_term_frame_idx {
                                if existing as u32 == idx {
                                    p.is_reference = false;
                                }
                            }
                        }
                    }
                    self.pictures.push(DecodedPicture {
                        long_term_frame_idx: Some(idx as u16),
                        ..*picture
                    });
                }
                4 => {
                    // max_long_term_frame_idx_plus1 - 1 之外的长期帧全部不用。
                    let limit = marking.max_long_term_frame_idx_plus1.saturating_sub(1);
                    for p in self.pictures.iter_mut() {
                        if p.is_reference && p.is_long_term() {
                            if let Some(existing) = p.long_term_frame_idx {
                                if existing as u32 > limit {
                                    p.is_reference = false;
                                }
                            }
                        }
                    }
                }
                5 => {
                    // 所有长期帧都不用。
                    for p in self.pictures.iter_mut() {
                        if p.is_reference && p.is_long_term() {
                            p.is_reference = false;
                        }
                    }
                    had_mmco5 = true;
                }
                _ => {}
            }
        }
        had_mmco5
    }

    /// 构造参考帧列表 0 与 1（spec 8.2.4）。
    ///
    /// 照抄 Chromium `H264Decoder::PrepareRefPicLists()` +
    /// `ModifyReferencePicLists()`：
    ///
    /// - **P / SP 切片**（spec 8.2.4.2.1）：列表 0 = 短期帧按 `pic_num`
    ///   **递减**（新的在前）+ 长期帧按 `long_term_pic_num` 递增；
    ///   列表 1 不用（留空）。
    /// - **B 切片**（spec 8.2.4.2.3 / 8.2.4.2.4）：列表 0 = POC 小于当前帧的
    ///   （按 POC 递减）+ POC 大于当前帧的（按 POC 递增）+ 长期帧；
    ///   列表 1 把前两段调换。两个列表完全相同时，交换列表 1 的前两项。
    ///
    /// 之后按切片头里的 `ref_pic_list_modification` 重排。
    ///
    /// `current_poc` 是当前图像的 POC —— B 切片的列表构造要拿它分组，
    /// 而当前图像此刻还没进 DPB，所以只能由调用方传进来。
    ///
    /// ⚠️ DXVA2 路径**不用**这个函数：驱动会按 `RefFrameList`（DPB 顺序）
    /// 自己构造参考列表。它在这里是为了与 Chromium 对照着看。
    pub fn reference_lists(
        &self,
        sps: &H264Sps,
        slice: &H264SliceHeader,
        current_poc: i32,
    ) -> Result<(Vec<DecodedPicture>, Vec<DecodedPicture>), H264ParseError> {
        let max_frame_num = Self::max_frame_num(sps) as i32;
        let current_frame_num = slice.frame_num as u16;

        let short_term: Vec<DecodedPicture> = self
            .pictures
            .iter()
            .copied()
            .filter(|p| p.is_reference && !p.is_long_term() && !p.non_existing)
            .collect();
        let mut long_term: Vec<DecodedPicture> = self
            .pictures
            .iter()
            .copied()
            .filter(|p| p.is_reference && p.is_long_term() && !p.non_existing)
            .collect();
        long_term.sort_by_key(|p| p.long_term_frame_idx.unwrap_or(0));

        let (mut list0, mut list1) = if slice.slice_type.is_b() {
            let mut before: Vec<DecodedPicture> = short_term
                .iter()
                .copied()
                .filter(|p| p.top_field_order_cnt < current_poc)
                .collect();
            before.sort_by_key(|p| std::cmp::Reverse(p.top_field_order_cnt));
            let mut after: Vec<DecodedPicture> = short_term
                .iter()
                .copied()
                .filter(|p| p.top_field_order_cnt > current_poc)
                .collect();
            after.sort_by_key(|p| p.top_field_order_cnt);

            let mut b0 = before.clone();
            b0.extend(after.iter().copied());
            b0.extend(long_term.iter().copied());

            let mut b1 = after.clone();
            b1.extend(before.iter().copied());
            b1.extend(long_term.iter().copied());

            // spec 8.2.4.2.3：两个列表完全相同时，交换列表 1 的前两项。
            if b1.len() > 1 && b0 == b1 {
                b1.swap(0, 1);
            }
            (b0, b1)
        } else {
            // P / SP 切片：短期帧按 `pic_num` 递减（新的在前）。
            let mut p0 = short_term;
            p0.sort_by_key(|p| {
                std::cmp::Reverse(pic_num(p.frame_num, current_frame_num, max_frame_num))
            });
            p0.extend(long_term.iter().copied());
            (p0, Vec::new())
        };

        if slice.ref_pic_list_modification_flag_l0 {
            list0 = apply_modifications(
                list0,
                &slice.ref_list_l0_modifications,
                current_frame_num,
                max_frame_num,
                &self.pictures,
            )?;
        }
        if slice.ref_pic_list_modification_flag_l1 {
            list1 = apply_modifications(
                list1,
                &slice.ref_list_l1_modifications,
                current_frame_num,
                max_frame_num,
                &self.pictures,
            )?;
        }
        Ok((list0, list1))
    }
}

/// `pic_num`（spec 8.2.4.1）：回绕过的帧号。
///
/// 对应 Chromium `H264Decoder::UpdatePicNums()`：`frame_num` 大于当前帧号
/// 时减去 `MaxFrameNum`（于是它排在「上一轮」），否则就是 `frame_num`。
fn pic_num(frame_num: u16, current_frame_num: u16, max_frame_num: i32) -> i32 {
    let frame_num = frame_num as i32;
    let current = current_frame_num as i32;
    if frame_num > current {
        frame_num - max_frame_num
    } else {
        frame_num
    }
}

/// 按 `ref_pic_list_modification` 重排参考帧列表（spec 8.2.4.3）。
///
/// 对应 Chromium `H264Decoder::ModifyReferencePicList()`。
///
/// ⚠️ 与 Chromium 一致：**引用到一张不存在的图片时直接失败**，而不是塞一个
/// 占位。规格允许放占位，但 Chromium 选择报错 —— 码流说它引用了一张不在
/// DPB 里的图，那就是码流坏了，报错比画出一张灰图有用。
fn apply_modifications(
    list: Vec<DecodedPicture>,
    modifications: &[crate::h264::parser::RefPicListModification],
    current_frame_num: u16,
    max_frame_num: i32,
    dpb: &[DecodedPicture],
) -> Result<Vec<DecodedPicture>, H264ParseError> {
    let mut result = list;
    let current_pic_num = current_frame_num as i32;
    // `pic_num_lx_pred`：上一条修改指令算出来的 `pic_num`（spec 8.2.4.3）。
    let mut pic_num_lx_pred = current_pic_num;
    let mut ref_idx_lx = 0usize;

    for modification in modifications {
        match modification.modification_of_pic_nums_idc {
            0 | 1 => {
                let delta = modification.abs_diff_pic_num_minus1 as i32 + 1;
                let mut no_wrap = if modification.modification_of_pic_nums_idc == 0 {
                    pic_num_lx_pred - delta
                } else {
                    pic_num_lx_pred + delta
                };
                if no_wrap < 0 {
                    no_wrap += max_frame_num;
                }
                if no_wrap >= max_frame_num {
                    no_wrap -= max_frame_num;
                }
                pic_num_lx_pred = no_wrap;
                let pic_num_lx = if no_wrap > current_pic_num {
                    no_wrap - max_frame_num
                } else {
                    no_wrap
                };
                let picture = dpb
                    .iter()
                    .copied()
                    .find(|p| {
                        p.is_reference
                            && !p.is_long_term()
                            && !p.non_existing
                            && pic_num(p.frame_num, current_frame_num, max_frame_num) == pic_num_lx
                    })
                    .ok_or(H264ParseError::InvalidStream)?;
                insert_at(&mut result, ref_idx_lx, &picture);
                ref_idx_lx += 1;
            }
            2 => {
                let picture = dpb
                    .iter()
                    .copied()
                    .find(|p| {
                        p.is_reference
                            && p.is_long_term()
                            && !p.non_existing
                            && p.long_term_frame_idx == Some(modification.long_term_pic_num as u16)
                    })
                    .ok_or(H264ParseError::InvalidStream)?;
                insert_at(&mut result, ref_idx_lx, &picture);
                ref_idx_lx += 1;
            }
            // idc 3 = 修改结束。
            _ => break,
        }
    }
    Ok(result)
}

/// 把一张图片插到列表的第 `index` 位（后面的往后挪）。
///
/// 对应 Chromium 的 `ShiftRightAndInsert()`，外加它后面那段去重：
/// 被挤到后面的同一张图要去掉（规格 8.2.4.3）。
fn insert_at(list: &mut Vec<DecodedPicture>, index: usize, picture: &DecodedPicture) {
    let index = index.min(list.len());
    list.insert(index, *picture);
    if let Some(duplicate) = list[index + 1..]
        .iter()
        .rposition(|p| p.picture_index == picture.picture_index)
    {
        list.remove(index + 1 + duplicate);
    }
    list.truncate(MAX_PICTURES);
}

/// MMCO 的辅助视图（给测试与日志用）。
pub fn mmco_summary(marking: &DecRefPicMarking) -> String {
    format!(
        "op={} diff={} lt_pic_num={} lt_idx={} max_lt_idx_plus1={}",
        marking.memory_management_control_operation,
        marking.difference_of_pic_nums_minus1,
        marking.long_term_pic_num,
        marking.long_term_frame_idx,
        marking.max_long_term_frame_idx_plus1
    )
}

#[cfg(test)]
#[path = "../../tests/unit/dxva2_dpb_tests.rs"]
mod tests;
