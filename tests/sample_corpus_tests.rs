use anyhow::{Context, Result, ensure};
use jpgfromraw::{FindJpegType, process_file_bytes};
use std::path::{Path, PathBuf};
use std::time::Instant;

struct SampleCase {
    extension: &'static str,
    manufacturer: &'static str,
    file_name: &'static str,
    source_url: &'static str,
}

const SAMPLE_CASES: &[SampleCase] = &[
    SampleCase {
        extension: "3fr",
        manufacturer: "Hasselblad",
        file_name: "hasselblad_cfv.3fr",
        source_url: "https://raw.pixls.us/getfile.php/709/nice/Hasselblad%20-%20CFV.3FR",
    },
    SampleCase {
        extension: "ari",
        manufacturer: "Arri",
        file_name: "arri_alexa_mini.ari",
        source_url: "https://raw.pixls.us/getfile.php/3497/nice/Arri%20-%20Alexa%20Mini.ari",
    },
    SampleCase {
        extension: "arw",
        manufacturer: "Sony",
        file_name: "sony_ilce7s_compressed.arw",
        source_url: "https://raw.pixls.us/getfile.php/1582/nice/Sony%20-%20ILCE-7S%20-%2014bit%2014bit%20compressed%20(3:2).ARW",
    },
    SampleCase {
        extension: "arw",
        manufacturer: "Sony",
        file_name: "sony_ilce_6300.arw",
        source_url: "https://raw.pixls.us/getfile.php/783/nice/Sony%20-%20ILCE-6300%20-%2012bit%2012bit%20compressed%20(3:2).ARW",
    },
    SampleCase {
        extension: "arw",
        manufacturer: "Sony",
        file_name: "sony_nex_7.arw",
        source_url: "https://raw.pixls.us/getfile.php/360/nice/Sony%20-%20NEX-7%20-%2012bit%2012bit%20compressed%20(3:2).ARW",
    },
    SampleCase {
        extension: "arw",
        manufacturer: "Sony",
        file_name: "sony_ilce_6000.arw",
        source_url: "https://raw.pixls.us/getfile.php/1971/nice/Sony%20-%20ILCE-6000%20-%2012bit%2012bit%20compressed%20(3:2).ARW",
    },
    SampleCase {
        extension: "arw",
        manufacturer: "Sony",
        file_name: "sony_ilce_6400.arw",
        source_url: "https://raw.pixls.us/getfile.php/3151/nice/Sony%20-%20ILCE-6400%20-%2014bit%2014bit%20compressed%20(3:2).ARW",
    },
    SampleCase {
        extension: "cr2",
        manufacturer: "Canon",
        file_name: "canon_eos40d_sraw2.cr2",
        source_url: "https://raw.pixls.us/getfile.php/2102/nice/Canon%20-%20EOS%2040D%20-%20sRAW2%20(sRAW)%20(3:2).CR2",
    },
    SampleCase {
        extension: "cr2",
        manufacturer: "Canon",
        file_name: "canon_eos_80d.cr2",
        source_url: "https://raw.pixls.us/getfile.php/1294/nice/Canon%20-%20EOS%2080D%20-%20RAW%20(3:2).CR2",
    },
    SampleCase {
        extension: "cr2",
        manufacturer: "Canon",
        file_name: "canon_eos_6d.cr2",
        source_url: "https://raw.pixls.us/getfile.php/799/nice/Canon%20-%20EOS%206D%20-%20RAW.CR2",
    },
    SampleCase {
        extension: "cr2",
        manufacturer: "Canon",
        file_name: "canon_eos_7d_sraw2.cr2",
        source_url: "https://raw.pixls.us/getfile.php/129/nice/Canon%20-%20EOS%207D%20-%20sRAW2%20(sRAW)%20(3:2).CR2",
    },
    SampleCase {
        extension: "cr2",
        manufacturer: "Canon",
        file_name: "canon_eos_7d_mark_ii.cr2",
        source_url: "https://raw.pixls.us/getfile.php/1064/nice/Canon%20-%20EOS%207D%20Mark%20II%20-%20RAW%20(3:2).cr2",
    },
    SampleCase {
        extension: "cr2",
        manufacturer: "Canon",
        file_name: "canon_eos_5d_mark_ii.cr2",
        source_url: "https://raw.pixls.us/getfile.php/1182/nice/Canon%20-%20EOS%205D%20Mark%20II%20-%20RAW%20(3:2).CR2",
    },
    SampleCase {
        extension: "cr2",
        manufacturer: "Canon",
        file_name: "canon_eos_5d_mark_iii.cr2",
        source_url: "https://raw.pixls.us/getfile.php/771/nice/Canon%20-%20EOS%205D%20Mark%20III.CR2",
    },
    SampleCase {
        extension: "cr2",
        manufacturer: "Canon",
        file_name: "canon_eos_5d_mark_iv.cr2",
        source_url: "https://raw.pixls.us/getfile.php/980/nice/Canon%20-%20EOS%205D%20Mark%20IV%20-%20RAW%20(3:2).CR2",
    },
    SampleCase {
        extension: "cr3",
        manufacturer: "Canon",
        file_name: "canon_eos_r6.cr3",
        source_url: "https://raw.pixls.us/getfile.php/4659/nice/Canon%20-%20EOS%20R6%20-%203:2.CR3",
    },
    SampleCase {
        extension: "cr3",
        manufacturer: "Canon",
        file_name: "canon_eos_r5.cr3",
        source_url: "https://raw.pixls.us/getfile.php/4692/nice/Canon%20-%20EOS%20R5%20-%203:2.CR3",
    },
    SampleCase {
        extension: "cr3",
        manufacturer: "Canon",
        file_name: "canon_eos_r7.cr3",
        source_url: "https://raw.pixls.us/getfile.php/5633/nice/Canon%20-%20EOS%20R7%20-%203:2.CR3",
    },
    SampleCase {
        extension: "cr3",
        manufacturer: "Canon",
        file_name: "canon_eos_90d.cr3",
        source_url: "https://raw.pixls.us/getfile.php/4649/nice/Canon%20-%20EOS%2090D%20-%203:2.CR3",
    },
    SampleCase {
        extension: "cr3",
        manufacturer: "Canon",
        file_name: "canon_eos_m50.cr3",
        source_url: "https://raw.pixls.us/getfile.php/4656/nice/Canon%20-%20EOS%20M50%20-%203:2.CR3",
    },
    SampleCase {
        extension: "crw",
        manufacturer: "Canon",
        file_name: "canon_powershot_g1.crw",
        source_url: "https://raw.pixls.us/getfile.php/2073/nice/Canon%20-%20PowerShot%20G1%20-%20RAW%20(4:3).crw",
    },
    SampleCase {
        extension: "crw",
        manufacturer: "Canon",
        file_name: "canon_powershot_g2.crw",
        source_url: "https://raw.pixls.us/getfile.php/695/nice/Canon%20-%20PowerShot%20G2.CRW",
    },
    SampleCase {
        extension: "crw",
        manufacturer: "Canon",
        file_name: "canon_powershot_g3.crw",
        source_url: "https://raw.pixls.us/getfile.php/684/nice/Canon%20-%20PowerShot%20G3.CRW",
    },
    SampleCase {
        extension: "crw",
        manufacturer: "Canon",
        file_name: "canon_powershot_pro70.crw",
        source_url: "https://raw.pixls.us/getfile.php/2070/nice/Canon%20-%20PowerShot%20Pro70%20-%20RAW%20(4:3).crw",
    },
    SampleCase {
        extension: "crw",
        manufacturer: "Canon",
        file_name: "canon_powershot_pro1.crw",
        source_url: "https://raw.pixls.us/getfile.php/555/nice/Canon%20-%20PowerShot%20Pro1.CRW",
    },
    SampleCase {
        extension: "crw",
        manufacturer: "Canon",
        file_name: "canon_powershot_s30.crw",
        source_url: "https://raw.pixls.us/getfile.php/611/nice/Canon%20-%20PowerShot%20S30.CRW",
    },
    SampleCase {
        extension: "cr2",
        manufacturer: "Canon",
        file_name: "canon_powershot_g9.cr2",
        source_url: "https://raw.pixls.us/getfile.php/760/nice/Canon%20-%20PowerShot%20G9%20-%20RAW%20(4:3).CR2",
    },
    SampleCase {
        extension: "cr2",
        manufacturer: "Canon",
        file_name: "canon_powershot_g10.cr2",
        source_url: "https://raw.pixls.us/getfile.php/170/nice/Canon%20-%20PowerShot%20G10%20-%20RAW%20(4:3).CR2",
    },
    SampleCase {
        extension: "cr2",
        manufacturer: "Canon",
        file_name: "canon_powershot_g16.cr2",
        source_url: "https://raw.pixls.us/getfile.php/1114/nice/Canon%20-%20PowerShot%20G16%20-%20RAW%20(4:3).CR2",
    },
    SampleCase {
        extension: "dcr",
        manufacturer: "Kodak",
        file_name: "kodak_dcs760c.dcr",
        source_url: "https://raw.pixls.us/getfile.php/1347/nice/Kodak%20-%20DCS760C%20-%2012bit%20%283%3A2%29.DCR",
    },
    SampleCase {
        extension: "dcr",
        manufacturer: "Kodak",
        file_name: "kodak_dcs_pro_14n.dcr",
        source_url: "https://raw.pixls.us/getfile.php/2970/nice/Kodak%20-%20DCS%20Pro%2014n%20-%2012bit%20(3:2).DCR",
    },
    SampleCase {
        extension: "dng",
        manufacturer: "Blackmagic",
        file_name: "blackmagic_microcinemacamera.dng",
        source_url: "https://raw.pixls.us/getfile.php/7317/nice/Blackmagic%20-%20Micro%20Cinema%20Camera%20-%2012bit%20(16:9).dng",
    },
    SampleCase {
        extension: "dng",
        manufacturer: "Apple",
        file_name: "apple_iphone_se.dng",
        source_url: "https://raw.pixls.us/getfile.php/1207/nice/Apple%20-%20iPhone%20SE%20-%2016bit%20(4:3).dng",
    },
    SampleCase {
        extension: "dng",
        manufacturer: "Apple",
        file_name: "apple_iphone_8.dng",
        source_url: "https://raw.pixls.us/getfile.php/2835/nice/Apple%20-%20iPhone%208%20-%2016bit%20(4:3).dng",
    },
    SampleCase {
        extension: "dng",
        manufacturer: "Apple",
        file_name: "apple_iphone_12_pro.dng",
        source_url: "https://raw.pixls.us/getfile.php/4264/nice/Apple%20-%20iPhone%2012%20Pro%20-%208bit%20(4:3).DNG",
    },
    SampleCase {
        extension: "dng",
        manufacturer: "Apple",
        file_name: "apple_iphone_12_pro_max.dng",
        source_url: "https://img.photographyblog.com/reviews/apple_iphone_12_pro_max/sample_images/apple_iphone_12_pro_max_01.dng",
    },
    SampleCase {
        extension: "dng",
        manufacturer: "Apple",
        file_name: "apple_iphone_13_pro.dng",
        source_url: "https://img.photographyblog.com/reviews/apple_iphone_13_pro/sample_images/apple_iphone_13_pro_01.dng",
    },
    SampleCase {
        extension: "dng",
        manufacturer: "Ricoh",
        file_name: "ricoh_gr.dng",
        source_url: "https://raw.pixls.us/getfile.php/996/nice/Ricoh%20-%20GR%20-%2012bit%20(4:3).DNG",
    },
    SampleCase {
        extension: "dng",
        manufacturer: "Ricoh",
        file_name: "ricoh_gr_ii.dng",
        source_url: "https://raw.pixls.us/getfile.php/1955/nice/Ricoh%20-%20GR%20II%20-%2012bit%20(3:2).DNG",
    },
    SampleCase {
        extension: "dng",
        manufacturer: "Ricoh",
        file_name: "ricoh_gr_iii.dng",
        source_url: "https://raw.pixls.us/getfile.php/3115/nice/Ricoh%20-%20GR%20III%20-%2014bit%20(3:2).DNG",
    },
    SampleCase {
        extension: "dng",
        manufacturer: "Leica",
        file_name: "leica_m9.dng",
        source_url: "https://raw.pixls.us/getfile.php/753/nice/Leica%20-%20M9%20-%208bit%208bit%20compressed%20(3:2).DNG",
    },
    SampleCase {
        extension: "erf",
        manufacturer: "Epson",
        file_name: "epson_rd1.erf",
        source_url: "https://raw.pixls.us/getfile.php/2680/nice/Epson%20-%20R-D1%20-%2012bit%20(3:2).ERF",
    },
    SampleCase {
        extension: "fff",
        manufacturer: "Hasselblad",
        file_name: "hasselblad_h4d40.fff",
        source_url: "https://raw.pixls.us/getfile.php/931/nice/Hasselblad%20-%20H4D-40%20-%2016bit%20%284%3A3%29.fff",
    },
    SampleCase {
        extension: "gpr",
        manufacturer: "GoPro",
        file_name: "gopro_fusion.gpr",
        source_url: "https://raw.pixls.us/getfile.php/7370/nice/GoPro%20-%20FUSION%20-%2016bit%20%281%3A1%29.GPR",
    },
    SampleCase {
        extension: "gpr",
        manufacturer: "GoPro",
        file_name: "gopro_hero5_black.gpr",
        source_url: "https://raw.pixls.us/getfile.php/7371/nice/GoPro%20-%20HERO5%20Black%20-%2016bit%20%284%3A3%29.GPR",
    },
    SampleCase {
        extension: "iiq",
        manufacturer: "Phase One",
        file_name: "phase_one_p40_plus.iiq",
        source_url: "https://raw.pixls.us/getfile.php/8010/nice/Phase%20One%20-%20P40%2B%20-%20IIQ%20S%20%284%3A3%29.IIQ",
    },
    SampleCase {
        extension: "kdc",
        manufacturer: "Kodak",
        file_name: "kodak_p712.kdc",
        source_url: "https://raw.pixls.us/getfile.php/2345/nice/Kodak%20-%20KODAK%20P712%20ZOOM%20DIGITAL%20CAMERA%20-%20NAN.KDC",
    },
    SampleCase {
        extension: "kdc",
        manufacturer: "Kodak",
        file_name: "kodak_dc120.kdc",
        source_url: "https://raw.pixls.us/getfile.php/2338/nice/Kodak%20-%20DC120%20-%208bit%20(4:3).kdc",
    },
    SampleCase {
        extension: "kdc",
        manufacturer: "Kodak",
        file_name: "kodak_p880.kdc",
        source_url: "https://raw.pixls.us/getfile.php/2339/nice/Kodak%20-%20P880%20-%20NAN.KDC",
    },
    SampleCase {
        extension: "mef",
        manufacturer: "Mamiya",
        file_name: "mamiya_zd.mef",
        source_url: "https://raw.pixls.us/getfile.php/562/nice/Mamiya%20-%20ZD.MEF",
    },
    SampleCase {
        extension: "mdc",
        manufacturer: "Minolta",
        file_name: "minolta_rd175.mdc",
        source_url: "https://raw.pixls.us/getfile.php/4510/nice/Minolta%20-%20RD%20175.MDC",
    },
    SampleCase {
        extension: "mos",
        manufacturer: "Leaf",
        file_name: "leaf_aptus22.mos",
        source_url: "https://raw.pixls.us/getfile.php/3196/nice/Leaf%20-%20Aptus%2022%20-%2016bit%20%284%3A3%29.mos",
    },
    SampleCase {
        extension: "mrw",
        manufacturer: "Minolta",
        file_name: "minolta_dimage_z2.mrw",
        source_url: "https://raw.pixls.us/getfile.php/373/nice/Minolta%20-%20Dimage%20Z2.MRW",
    },
    SampleCase {
        extension: "nef",
        manufacturer: "Nikon",
        file_name: "nikon_coolscan_iv_ed.nef",
        source_url: "https://raw.pixls.us/getfile.php/4282/nice/Nikon%20-%20Nikon%20COOLSCAN%20IV%20ED%20-%20uncompressed%20(4:3).nef",
    },
    SampleCase {
        extension: "nef",
        manufacturer: "Nikon",
        file_name: "nikon_d200.nef",
        source_url: "https://raw.pixls.us/getfile.php/824/nice/Nikon%20-%20D200%20-%2012bit%2012bit%20uncompressed%20(3:2).nef",
    },
    SampleCase {
        extension: "nef",
        manufacturer: "Nikon",
        file_name: "nikon_d7000.nef",
        source_url: "https://raw.pixls.us/getfile.php/958/nice/Nikon%20-%20D7000%20-%2014bit%2014bit%20compressed%20(Lossless)%20(3:2).NEF",
    },
    SampleCase {
        extension: "nef",
        manufacturer: "Nikon",
        file_name: "nikon_d750.nef",
        source_url: "https://raw.pixls.us/getfile.php/895/nice/Nikon%20-%20D750%20-%2012bit%2012bit%20compressed%20(Lossy%20(type%202))%20(3:2).NEF",
    },
    SampleCase {
        extension: "nef",
        manufacturer: "Nikon",
        file_name: "nikon_d800.nef",
        source_url: "https://raw.pixls.us/getfile.php/750/nice/Nikon%20-%20D800%20-%2014bit%2014bit%20compressed%20(Lossless)%20(3:2).NEF",
    },
    SampleCase {
        extension: "nef",
        manufacturer: "Nikon",
        file_name: "nikon_d850.nef",
        source_url: "https://raw.pixls.us/getfile.php/1835/nice/Nikon%20-%20D850%20-%2012bit%2012bit%20compressed%20(Lossless)%20(3:2).NEF",
    },
    SampleCase {
        extension: "nrw",
        manufacturer: "Nikon",
        file_name: "nikon_coolpix_p7000.nrw",
        source_url: "https://raw.pixls.us/getfile.php/598/nice/Nikon%20-%20Coolpix%20P7000%20-%2012bit%2012bit-uncompressed.NRW",
    },
    SampleCase {
        extension: "nrw",
        manufacturer: "Nikon",
        file_name: "nikon_coolpix_p330.nrw",
        source_url: "https://raw.pixls.us/getfile.php/912/nice/Nikon%20-%20Coolpix%20P330%20-%2012bit%2012bit%20compressed%20((7))%20(4:3).NRW",
    },
    SampleCase {
        extension: "nrw",
        manufacturer: "Nikon",
        file_name: "nikon_coolpix_p6000.nrw",
        source_url: "https://raw.pixls.us/getfile.php/651/nice/Nikon%20-%20Coolpix%20P6000%20-%2012bit%2012bit-uncompressed.NRW",
    },
    SampleCase {
        extension: "nrw",
        manufacturer: "Nikon",
        file_name: "nikon_coolpix_p7100.nrw",
        source_url: "https://raw.pixls.us/getfile.php/585/nice/Nikon%20-%20Coolpix%20P7100%20-%2012bit%2012bit-uncompressed.NRW",
    },
    SampleCase {
        extension: "orf",
        manufacturer: "Olympus",
        file_name: "olympus_c5050z.orf",
        source_url: "https://raw.pixls.us/getfile.php/254/nice/Olympus%20-%20C5050Z%20-%2012bit%20(4:3).ORF",
    },
    SampleCase {
        extension: "orf",
        manufacturer: "Olympus",
        file_name: "olympus_em1.orf",
        source_url: "https://raw.pixls.us/getfile.php/1051/nice/Olympus%20-%20E-M1%20-%2016bit%20(4:3).orf",
    },
    SampleCase {
        extension: "orf",
        manufacturer: "Olympus",
        file_name: "olympus_em5.orf",
        source_url: "https://raw.pixls.us/getfile.php/1084/nice/Olympus%20-%20E-M5%20-%2016bit%20(4:3).ORF",
    },
    SampleCase {
        extension: "orf",
        manufacturer: "Olympus",
        file_name: "olympus_em10.orf",
        source_url: "https://raw.pixls.us/getfile.php/1331/nice/Olympus%20-%20E-M10%20-%2016bit%20(4:3).ORF",
    },
    SampleCase {
        extension: "orf",
        manufacturer: "Olympus",
        file_name: "olympus_em5_mark_ii.orf",
        source_url: "https://raw.pixls.us/getfile.php/2856/nice/Olympus%20-%20E-M5%20Mark%20II%20-%2016bit%2016bit%20%22high-res%22%20(4:3).ORF",
    },
    SampleCase {
        extension: "orf",
        manufacturer: "Olympus",
        file_name: "olympus_em1_mark_ii.orf",
        source_url: "https://img.photographyblog.com/reviews/olympus_om_d_e_m1_mark_ii/photos/olympus_om_d_e_m1_mark_ii_01.orf",
    },
    SampleCase {
        extension: "pef",
        manufacturer: "Pentax",
        file_name: "pentax_k10d.pef",
        source_url: "https://raw.pixls.us/getfile.php/2239/nice/Pentax%20-%20K10D%20-%2012bit%2012bit%20compressed%20(3:2).PEF",
    },
    SampleCase {
        extension: "pef",
        manufacturer: "Pentax",
        file_name: "pentax_k_5_ii.pef",
        source_url: "https://raw.pixls.us/getfile.php/874/nice/Pentax%20-%20K-5%20II%20-%2014bit%20(3:2).PEF",
    },
    SampleCase {
        extension: "pef",
        manufacturer: "Pentax",
        file_name: "pentax_k_3.pef",
        source_url: "https://raw.pixls.us/getfile.php/1075/nice/Pentax%20-%20K-3%20-%2014bit%20(3:2).PEF",
    },
    SampleCase {
        extension: "pef",
        manufacturer: "Pentax",
        file_name: "pentax_k_7.pef",
        source_url: "https://raw.pixls.us/getfile.php/828/nice/Pentax%20-%20K-7%20-%2012bit%20(3:2).PEF",
    },
    SampleCase {
        extension: "raf",
        manufacturer: "Fujifilm",
        file_name: "fujifilm_finepix_s5000.raf",
        source_url: "https://raw.pixls.us/getfile.php/2726/nice/Fujifilm%20-%20FinePix%20S5000%20-%204:3.RAF",
    },
    SampleCase {
        extension: "raf",
        manufacturer: "Fujifilm",
        file_name: "fujifilm_x_t2.raf",
        source_url: "https://raw.pixls.us/getfile.php/864/nice/Fujifilm%20-%20X-T2%20-%2014bit%2014bit%20uncompressed%20(3:2).RAF",
    },
    SampleCase {
        extension: "raf",
        manufacturer: "Fujifilm",
        file_name: "fujifilm_x_t3.raf",
        source_url: "https://raw.pixls.us/getfile.php/2783/nice/Fujifilm%20-%20X-T3%20-%2014bit%2014bit%20compressed%20(3:2).RAF",
    },
    SampleCase {
        extension: "raw",
        manufacturer: "Kodak",
        file_name: "kodak_c330.raw",
        source_url: "https://raw.pixls.us/getfile.php/590/nice/Kodak%20-%20Kodak%20C330%20-%20Format%204:2:2%20YRGB%20(4:2).RAW",
    },
    SampleCase {
        extension: "raw",
        manufacturer: "Pentax",
        file_name: "pentax_optio_s4.raw",
        source_url: "http://www.rawsamples.ch/raws/pentax/optios4/RAW_PENTAX_OPTIOS4.RAW",
    },
    SampleCase {
        extension: "rw2",
        manufacturer: "Panasonic",
        file_name: "panasonic_dmc_lx7.rw2",
        source_url: "https://raw.pixls.us/getfile.php/7008/nice/Panasonic%20-%20DMC-LX7%20-%201:1.RW2",
    },
    SampleCase {
        extension: "rw2",
        manufacturer: "Panasonic",
        file_name: "panasonic_dmc_gf1.rw2",
        source_url: "https://raw.pixls.us/getfile.php/880/nice/Panasonic%20-%20DMC-GF1%20-%204:3.rw2",
    },
    SampleCase {
        extension: "rw2",
        manufacturer: "Panasonic",
        file_name: "panasonic_dmc_gh4.rw2",
        source_url: "https://raw.pixls.us/getfile.php/2570/nice/Panasonic%20-%20DMC-GH4%20-%203:2.RW2",
    },
    SampleCase {
        extension: "rw2",
        manufacturer: "Panasonic",
        file_name: "panasonic_dmc_gx7.rw2",
        source_url: "https://raw.pixls.us/getfile.php/282/nice/Panasonic%20-%20DMC-GX7%20-%204:3.RW2",
    },
    SampleCase {
        extension: "rw2",
        manufacturer: "Panasonic",
        file_name: "panasonic_dmc_lx5.rw2",
        source_url: "https://raw.pixls.us/getfile.php/647/nice/Panasonic%20-%20DMC-LX5%20-%204:3.rw2",
    },
    SampleCase {
        extension: "rwl",
        manufacturer: "Leica",
        file_name: "leica_d_lux5.rwl",
        source_url: "https://raw.pixls.us/getfile.php/2811/nice/Leica%20-%20D-LUX%205%20-%201:1.RWL",
    },
    SampleCase {
        extension: "sr2",
        manufacturer: "Sony",
        file_name: "sony_dsc_r1.sr2",
        source_url: "https://raw.pixls.us/getfile.php/3221/nice/Sony%20-%20DSC-R1%20-%2014bit%2014bit%20uncompressed%20(3:2).SR2",
    },
    SampleCase {
        extension: "srf",
        manufacturer: "Sony",
        file_name: "sony_dsc_f828.srf",
        source_url: "https://raw.pixls.us/getfile.php/1351/nice/Sony%20-%20DSC-F828%20-%204:3.SRF",
    },
    SampleCase {
        extension: "srw",
        manufacturer: "Samsung",
        file_name: "samsung_nx500.srw",
        source_url: "https://raw.pixls.us/getfile.php/3668/nice/Samsung%20-%20NX500%20-%2012bit%2012bit%20normal%20compression%20(3:2).SRW",
    },
    SampleCase {
        extension: "srw",
        manufacturer: "Samsung",
        file_name: "samsung_nx1.srw",
        source_url: "https://raw.pixls.us/getfile.php/837/nice/Samsung%20-%20NX1%20-%2014bit%20(3:2).srw",
    },
    SampleCase {
        extension: "srw",
        manufacturer: "Samsung",
        file_name: "samsung_nx100.srw",
        source_url: "https://raw.pixls.us/getfile.php/556/nice/Samsung%20-%20NX100%20-%2012bit%20(3:2).SRW",
    },
    SampleCase {
        extension: "srw",
        manufacturer: "Samsung",
        file_name: "samsung_nx200.srw",
        source_url: "https://raw.pixls.us/getfile.php/839/nice/Samsung%20-%20NX200%20-%2012bit%20(3:2).srw",
    },
    SampleCase {
        extension: "srw",
        manufacturer: "Samsung",
        file_name: "samsung_nx300m.srw",
        source_url: "https://raw.pixls.us/getfile.php/715/nice/Samsung%20-%20NX300M%20-%2012bit%20(3:2).SRW",
    },
    SampleCase {
        extension: "x3f",
        manufacturer: "Polaroid",
        file_name: "polaroid_x530.x3f",
        source_url: "https://raw.pixls.us/getfile.php/584/nice/Polaroid%20-%20Polaroid%20x530.X3F",
    },
    SampleCase {
        extension: "tif",
        manufacturer: "Kodak",
        file_name: "kodak_dcs520c.tif",
        source_url: "https://raw.pixls.us/getfile.php/2573/nice/Kodak%20-%20DCS520C%20-%2012bit%20(3:2).TIF",
    },
    SampleCase {
        extension: "tif",
        manufacturer: "Kodak",
        file_name: "kodak_dcs560c.tif",
        source_url: "https://raw.pixls.us/getfile.php/581/nice/Kodak%20-%20DCS560C.TIF",
    },
];

fn sample_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("sample_raws")
}

fn sample_path(case: &SampleCase) -> PathBuf {
    sample_root().join(case.file_name)
}

fn decode_jpeg(bytes: &[u8]) -> Result<()> {
    ensure!(bytes.starts_with(&[0xFF, 0xD8]), "missing JPEG SOI marker");
    ensure!(bytes.ends_with(&[0xFF, 0xD9]), "missing JPEG EOI marker");
    let mut cursor = 2;
    let mut found_sof = false;

    while cursor + 3 < bytes.len() {
        ensure!(bytes[cursor] == 0xFF, "expected JPEG marker prefix");
        while cursor < bytes.len() && bytes[cursor] == 0xFF {
            cursor += 1;
        }
        ensure!(cursor < bytes.len(), "truncated JPEG marker");

        let marker = bytes[cursor];
        cursor += 1;

        if marker == 0xD9 {
            break;
        }
        if marker == 0xDA {
            ensure!(found_sof, "JPEG scan started before SOF");
            break;
        }
        if (0xD0..=0xD7).contains(&marker) || marker == 0x01 {
            continue;
        }

        ensure!(cursor + 1 < bytes.len(), "truncated JPEG segment length");
        let segment_len = u16::from_be_bytes([bytes[cursor], bytes[cursor + 1]]) as usize;
        ensure!(segment_len >= 2, "invalid JPEG segment length");
        let segment_start = cursor + 2;
        let segment_end = cursor + segment_len;
        ensure!(segment_end <= bytes.len(), "JPEG segment exceeds buffer");

        let is_sof = is_sof_marker(marker);
        if is_sof {
            ensure!(segment_end - segment_start >= 6, "truncated SOF segment");
            let precision = bytes[segment_start];
            let height = u16::from_be_bytes([bytes[segment_start + 1], bytes[segment_start + 2]]);
            let width = u16::from_be_bytes([bytes[segment_start + 3], bytes[segment_start + 4]]);
            ensure!(precision > 0, "JPEG precision must be positive");
            ensure!(width > 0, "JPEG width must be positive");
            ensure!(height > 0, "JPEG height must be positive");
            found_sof = true;
        }

        cursor = segment_end;
    }

    ensure!(found_sof, "missing JPEG SOF segment");
    Ok(())
}

/// The corpus is too large to commit, so these tests only run on machines that have downloaded
/// it; see SAMPLE_CASES for the source URLs.
fn corpus_available() -> bool {
    if sample_root().exists() {
        return true;
    }
    eprintln!(
        "skipping: sample corpus not present at {}",
        sample_root().display()
    );
    false
}

#[tokio::test]
async fn sample_corpus_files_exist() -> Result<()> {
    if !corpus_available() {
        return Ok(());
    }
    for case in SAMPLE_CASES {
        let path = sample_path(case);
        ensure!(
            path.exists(),
            "missing sample for .{}: {} ({})",
            case.extension,
            path.display(),
            case.source_url
        );
    }

    Ok(())
}

#[tokio::test]
async fn sample_corpus_extracts_valid_smallest_jpegs() -> Result<()> {
    if !corpus_available() {
        return Ok(());
    }
    for case in SAMPLE_CASES {
        let path = sample_path(case);
        let jpeg = process_file_bytes(&path, FindJpegType::Smallest)
            .await
            .with_context(|| format!("extract smallest JPEG for {}", case.file_name))?;
        decode_jpeg(&jpeg)
            .with_context(|| format!("decode smallest JPEG for {}", case.file_name))?;
    }

    Ok(())
}

#[tokio::test]
async fn sample_corpus_extracts_valid_largest_jpegs() -> Result<()> {
    if !corpus_available() {
        return Ok(());
    }
    for case in SAMPLE_CASES {
        let path = sample_path(case);
        let jpeg = process_file_bytes(&path, FindJpegType::Largest)
            .await
            .with_context(|| format!("extract largest JPEG for {}", case.file_name))?;
        decode_jpeg(&jpeg)
            .with_context(|| format!("decode largest JPEG for {}", case.file_name))?;
    }

    Ok(())
}

#[tokio::test]
async fn sample_corpus_largest_is_not_smaller_than_smallest() -> Result<()> {
    if !corpus_available() {
        return Ok(());
    }
    for case in SAMPLE_CASES {
        let path = sample_path(case);
        let smallest = process_file_bytes(&path, FindJpegType::Smallest).await?;
        let largest = process_file_bytes(&path, FindJpegType::Largest).await?;
        ensure!(
            largest.len() >= smallest.len(),
            "expected largest JPEG to be at least as large as smallest for {}",
            case.file_name
        );
    }

    Ok(())
}

#[tokio::test]
#[ignore = "benchmark-style audit for manual performance comparison across raw types"]
async fn sample_corpus_performance_audit() -> Result<()> {
    for case in SAMPLE_CASES {
        let path = sample_path(case);
        let input_size = std::fs::metadata(&path)?.len();

        let start = Instant::now();
        let jpeg = process_file_bytes(&path, FindJpegType::Smallest).await?;
        let elapsed = start.elapsed();

        println!(
            "{:>4} {:<12} input={:>8.2} MiB output={:>8.2} KiB elapsed={:>8.3} ms source={}",
            case.extension,
            case.manufacturer,
            input_size as f64 / (1024.0 * 1024.0),
            jpeg.len() as f64 / 1024.0,
            elapsed.as_secs_f64() * 1000.0,
            case.file_name,
        );
    }

    Ok(())
}

const fn is_sof_marker(marker: u8) -> bool {
    marker >= 0xC0 && marker <= 0xCF && !matches!(marker, 0xC4 | 0xC8 | 0xCC)
}
