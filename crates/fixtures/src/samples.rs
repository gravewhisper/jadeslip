use crate::epub::escape;
use crate::rng::derive;
use crate::zip::{DEFLATE, STORED, ZipWriter};
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};

pub struct Sample {
    pub file: &'static str,
    pub title: &'static str,
    pub author: &'static str,
    pub chapters: &'static [Chapter],
}

pub struct Chapter {
    pub title: &'static str,
    pub paragraphs: &'static [&'static str],
}

pub const SAMPLES: &[Sample] = &[
    Sample {
        file: "the-lantern-keeper",
        title: "The Lantern Keeper",
        author: "Edda Morrow",
        chapters: &[
            Chapter {
                title: "The Lamp on Harrow Point",
                paragraphs: &[
                    "Every evening, a little before the light failed, Nell climbed the hundred and twelve steps of the Harrow Point lighthouse and lit the lamp that had not gone dark in forty years.",
                    "She counted the steps out loud, the way her grandmother had taught her. Not because she might lose count, but because the counting kept the wind out of her head. Up here the wind had opinions, and most of them were about leaving.",
                    "The lamp room smelled of brass polish and old smoke. Nell trimmed the wick, wiped the lens with the soft cloth she kept in her coat, and struck a match. The flame leaned, steadied, and then stood up straight, as if it had remembered its job.",
                    "Far below, the sea turned silver wherever the beam touched it. Somewhere out there a boat she would never meet changed its course by half a degree and did not find the rocks. That was the whole of the work. It was enough.",
                    "When the lamp was settled she sat on the iron bench by the window and opened the logbook. The last entry before her own was in her grandmother's hand, dated eleven winters ago: _Fog from the north. Kept the light. Nothing else to say._",
                    "Nell wrote the date beneath it, then the wind, then the sea. She had never found anything else to say either, and she had begun to suspect that this was the point.",
                    "At midnight she climbed down again, counting backwards, and left the lamp to its long conversation with the dark.",
                ],
            },
            Chapter {
                title: "The Unsigned Letter",
                paragraphs: &[
                    "The letter was waiting on the doorstep in the morning, held down by a flat gray stone so the wind could not take it. It had no stamp and no return address, only her name in careful, upright capitals.",
                    "Nell turned it over twice before she opened it. She had not recieved a letter at the lighthouse in all the years she had kept it, and she was not sure she wanted to start now.",
                    "Inside was a single sheet, folded into thirds, and a brass key no longer than her thumb. _The light is not the only thing your grandmother kept,_ the letter said. _Look under the forty-first step._",
                    "She read it again in the kitchen, and a third time by the window, as if the words might settle into something more ordinary if she gave them the chance. They did not. The key sat on the table between her hands and caught the morning like a small, patient flame.",
                ],
            },
            Chapter {
                title: "Under the Loose Step",
                paragraphs: &[
                    "She had always known the forty-first step was loose. Everyone who climbed the tower knew it, because it rocked under your boot and made a sound like a cough. Nell had simply never asked why.",
                    "With a kitchen knife and some patience she lifted the iron plate. Beneath it, wrapped in oilcloth, was a tin box with a lock the size of a fingernail. The brass key turned as if it had been waiting for her.",
                    "Inside the box were eleven logbooks she had never seen, bound in green cloth, their corners soft with handling. The first was dated the year her grandmother came to Harrow Point. The last stopped in the middle of a sentence.",
                ],
            },
            Chapter {
                title: "Eleven Logbooks",
                paragraphs: &[
                    "The green logbooks did not record the weather. They recorded the boats. Each one had a name, a date, and a line about what the light had seen: _came in low under the cliff, lamp held, all hands home_.",
                    "Some entries were only a word. Others ran for pages, written in a hurry by the look of the ink, about nights when the fog had come down so thick that the beam stopped a few yards from the glass and her grandmother had rung the bell by hand until morning.",
                    "Nell read them on the iron bench, one logbook a night, while the lamp turned above her. By the ninth she understood that the lighthouse had never been a building that kept a light. It was a promise that somebody would always be awake.",
                ],
            },
            Chapter {
                title: "The Night of the Fog",
                paragraphs: &[
                    "The fog came on the last night of the year, as the green logbooks had said it sometimes would. It rolled in from the north and swallowed the beam whole, and the bell rope was stiff with salt when Nell found it.",
                    "She rang it until her arms burned and then rang it some more. Somewhere below, an engine slowed, hesitated, and turned away from the rocks. She heard it go, and she kept ringing until the fog thinned into an ordinary gray dawn.",
                    "Then she climbed the hundred and twelve steps, opened the newest logbook to a clean page, and finished her grandmother's sentence.",
                ],
            },
        ],
    },
    Sample {
        file: "quiet-engines",
        title: "Quiet Engines",
        author: "Tobias Wren",
        chapters: &[
            Chapter {
                title: "The Town That Stopped",
                paragraphs: &[
                    "In the town of Ferrow every machine had stopped at once, at seven minutes past four on a Tuesday afternoon. The mill wheel hung in the race. The tram stood halfway up the hill with its bell raised. The clock in the square kept a polite silence.",
                    "Nobody was hurt. People simply walked home, lit candles, and waited for someone to come and explain it to them. After a month, someone did.",
                ],
            },
            Chapter {
                title: "A Mechanic with No Tools",
                paragraphs: &[
                    "Iva arrived with a single canvas bag and no tools at all, which the town council found alarming. She asked for a chair, a cup of tea, and an hour alone with the tram.",
                    "She did not touch it. She sat beside it and listened, her head tilted, the way a doctor listens to a cough. At the end of the hour she said that the tram was not broken. It was waiting.",
                ],
            },
            Chapter {
                title: "What the Mill Wheel Wanted",
                paragraphs: &[
                    "The mill wheel, Iva said, had been turning for ninety years without once being thanked. She said it with such a straight face that the miller went red and then, after a long pause, went down to the race and said thank you.",
                    "Nothing happened. Iva nodded as if that was exactly what she had expected and wrote something in a small notebook.",
                ],
            },
            Chapter {
                title: "The Clock in the Square",
                paragraphs: &[
                    "The clock was the oldest machine in Ferrow, and the one Iva spent the longest beside. She slept under it for three nights. On the fourth morning she asked the town to bring every broken thing it owned to the square.",
                    "They came with kettles and music boxes, pocket watches and a child's tin bird. Iva laid them on the cobbles in a wide ring around the clock and told everyone to be quiet and wait.",
                ],
            },
            Chapter {
                title: "Seven Minutes Past Four",
                paragraphs: &[
                    "At seven minutes past four the clock moved. One tick, very small, like someone clearing their throat. Then the tin bird flapped, the kettles sighed, and up on the hill the tram rang its bell.",
                    "Iva was already walking to the station with her canvas bag. When the mayor ran after her to ask what she had done, she said she had only reminded the town how to listen. Machines, she said, are mostly patience.",
                ],
            },
        ],
    },
    Sample {
        file: "ninth-moon-cultivator",
        title: "Ninth Moon Cultivator",
        author: "Lan Shuyi",
        chapters: &[
            Chapter {
                title: "The Outer Courtyard",
                paragraphs: &[
                    "Wei Lin swept the outer courtyard of Cloudrest Sect for three years before anyone taught him a single breathing form. He swept in snow and in summer rain, and he learned the courtyard's stones better than he knew his own hands.",
                    "The disciples who passed him on their way to the training halls did not look down. A broom, they reasoned, was not a path to the heavens.",
                ],
            },
            Chapter {
                title: "A Crack in the Ninth Stone",
                paragraphs: &[
                    "On the first night of the ninth moon, Wei Lin noticed that the ninth stone from the gate had cracked. Moonlight gathered in the crack like water in a cupped hand, and when he swept across it the broom grew warm.",
                    "He told no one. He came back the next night, and the next, and swept the same stone until his breath began to move with the bristles.",
                ],
            },
            Chapter {
                title: "The Elder Who Did Not Sleep",
                paragraphs: &[
                    "Elder Qiao had not slept in forty years, and so she was the only one awake to see the outer disciple sweeping moonlight. She watched for a week before she came down from the tower.",
                    "She asked him who had taught him the Ninth Moon Breath. He said that nobody had, and that he was only trying to keep the courtyard clean. Elder Qiao laughed for the first time in a decade.",
                ],
            },
            Chapter {
                title: "The Sect Trial",
                paragraphs: &[
                    "At the spring trial Wei Lin stood in the arena holding his broom. The inner disciples smiled behind their sleeves. Their swords hummed with borrowed lightning.",
                    "He did not strike anyone. He swept the arena floor in one long, unhurried circle, and the lightning followed the bristles out of the ring and into the evening sky, where it went quietly to sleep.",
                ],
            },
            Chapter {
                title: "Sweeping the Sky",
                paragraphs: &[
                    "They offered him a room in the inner halls and a sword of his own. He took the room, because it was cold in the gatehouse, and gave the sword to the next boy who arrived with nothing.",
                    "Every ninth moon after that, the disciples of Cloudrest woke before dawn to the sound of a broom in the outer courtyard, and nobody passed it without looking down.",
                ],
            },
        ],
    },
    Sample {
        file: "a-glass-orchard",
        title: "A Glass Orchard",
        author: "Petra Ilse",
        chapters: &[
            Chapter {
                title: "Seeds in a Jam Jar",
                paragraphs: &[
                    "The seeds came in a jam jar with a paper label that said only _plant in full sun, water with care, do not drop_. Hanne found it in her aunt's pantry behind the pickled beets, and she planted all twelve before she thought to wonder what they were.",
                    "By midsummer the first saplings had come up clear as window panes. When the wind blew through them they rang.",
                ],
            },
            Chapter {
                title: "The First Frost",
                paragraphs: &[
                    "Frost was the danger. Hanne learned that on the first cold night, when one of the young trees cracked from root to crown with a sound like a dropped plate.",
                    "After that she wrapped them in wool every evening and unwrapped them every morning, and the neighbors stopped asking what she was doing and started bringing blankets.",
                ],
            },
            Chapter {
                title: "Fruit That Holds the Light",
                paragraphs: &[
                    "The fruit came in the third year: small pears of green glass that held whatever light they were picked in. A pear picked at noon glowed white all night. A pear picked at sunset kept the sunset.",
                    "Hanne picked one at every hour of a single day and lined them up on the kitchen windowsill, so that the kitchen was never entirely dark again.",
                ],
            },
            Chapter {
                title: "A Buyer from the City",
                paragraphs: &[
                    "A buyer came up from the city in a pale car and offered her more money than the farm had made in a century. He wanted the trees, roots and all, to stand in the lobby of a bank.",
                    "Hanne walked him through the orchard at dusk so he could hear it ringing. Then she gave him one pear, picked that minute, and sent him home.",
                ],
            },
            Chapter {
                title: "Harvest Moon",
                paragraphs: &[
                    "On the night of the harvest moon the whole village came to pick. They took the pears home in baskets and set them in windows up and down the valley.",
                    "From the hill road, travelers said, it looked as though the valley had caught a little of the moon and decided to keep it.",
                ],
            },
        ],
    },
    Sample {
        file: "the-salt-roads-north",
        title: "The Salt Roads North",
        author: "Corin Vass",
        chapters: &[
            Chapter {
                title: "The Last Caravan of Autumn",
                paragraphs: &[
                    "The salt caravans left the coast in spring and came home before the first snow. The last one of the year was always the slowest, because it carried everything the others had forgotten.",
                    "Ysolde joined it at the edge of the flats with a mule, a borrowed coat, and a map her father had drawn on the back of a tide table.",
                ],
            },
            Chapter {
                title: "Brine and Iron",
                paragraphs: &[
                    "The road north was paved with old salt, packed so hard by centuries of hooves that it rang like iron. At night the drovers sang to keep the cold out of their teeth.",
                    "Ysolde learned their songs by the third evening and their names by the fifth. She did not tell them why she was going north, and none of them asked.",
                ],
            },
            Chapter {
                title: "The Toll at White Bridge",
                paragraphs: &[
                    "At White Bridge the toll keeper wanted a sack of salt for every wagon. The caravan master argued, and the toll keeper raised the price, and the afternoon wore down into a sullen dusk.",
                    "Ysolde unfolded her father's map. On the back, in pencil, he had written the toll keeper's name and a debt that had never been paid. The caravan crossed for nothing.",
                ],
            },
            Chapter {
                title: "Snow Before Morning",
                paragraphs: &[
                    "The snow came early. The caravan woke under a white silence, with the road gone and the mules standing patient in drifts up to their bellies.",
                    "They dug for two days. On the second night, around the fire, Ysolde finally told them where she was going: to the salt spring her father had found and never gone back to.",
                ],
            },
            Chapter {
                title: "Where the Road Ends",
                paragraphs: &[
                    "The road ended at a frozen lake with a single spring at its heart, steaming in the cold. The water tasted of the sea, a hundred leagues from any shore.",
                    "Ysolde filled a small jar and drew the spring on the back of her father's map, beside his pencil marks. Then she turned the mule around and followed the caravan home.",
                ],
            },
        ],
    },
];

pub fn samples_dir() -> PathBuf {
    crate::generated_dir().join("samples")
}

pub fn generate(dir: &Path) -> io::Result<Vec<PathBuf>> {
    fs::create_dir_all(dir)?;
    SAMPLES.iter().map(|sample| write(sample, dir)).collect()
}

fn write(sample: &Sample, dir: &Path) -> io::Result<PathBuf> {
    let path = dir.join(format!("{}.epub", sample.file));
    let temp = dir.join(format!(".{}.epub.tmp", sample.file));
    let mut zip = ZipWriter::new(File::create(&temp)?);
    zip.add(b"mimetype", b"application/epub+zip", STORED)?;
    zip.add(b"META-INF/container.xml", CONTAINER.as_bytes(), DEFLATE)?;
    zip.add(b"OEBPS/content.opf", opf(sample).as_bytes(), DEFLATE)?;
    zip.add(b"OEBPS/nav.xhtml", nav(sample).as_bytes(), DEFLATE)?;
    for (ix, chapter) in sample.chapters.iter().enumerate() {
        let name = format!("OEBPS/{}", href(ix));
        zip.add(name.as_bytes(), page(chapter).as_bytes(), DEFLATE)?;
    }
    zip.finish()?;
    fs::rename(&temp, &path)?;
    Ok(path)
}

const CONTAINER: &str = "<?xml version=\"1.0\"?>\n<container version=\"1.0\" xmlns=\"urn:oasis:names:tc:opendocument:xmlns:container\"><rootfiles><rootfile full-path=\"OEBPS/content.opf\" media-type=\"application/oebps-package+xml\"/></rootfiles></container>\n";

fn href(ix: usize) -> String {
    format!("Text/ch{:02}.xhtml", ix + 1)
}

fn opf(sample: &Sample) -> String {
    let (a, b) = (derive(sample.file, 1), derive(sample.file, 2));
    let mut out = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<package xmlns=\"http://www.idpf.org/2007/opf\" version=\"3.0\" unique-identifier=\"uid\">\n<metadata xmlns:dc=\"http://purl.org/dc/elements/1.1/\">\n<dc:identifier id=\"uid\">urn:uuid:{:08x}-{:04x}-4{:03x}-8{:03x}-{:012x}</dc:identifier>\n<dc:title>{}</dc:title>\n<dc:creator>{}</dc:creator>\n<dc:language>en</dc:language>\n<meta property=\"dcterms:modified\">2026-01-01T00:00:00Z</meta>\n</metadata>\n<manifest>\n<item id=\"nav\" href=\"nav.xhtml\" media-type=\"application/xhtml+xml\" properties=\"nav\"/>\n",
        a >> 32,
        (a >> 16) & 0xFFFF,
        a & 0xFFF,
        (b >> 48) & 0xFFF,
        b & 0xFFFF_FFFF_FFFF,
        escape(sample.title),
        escape(sample.author),
    );
    for ix in 0..sample.chapters.len() {
        out.push_str(&format!(
            "<item id=\"ch{:02}\" href=\"{}\" media-type=\"application/xhtml+xml\"/>\n",
            ix + 1,
            href(ix)
        ));
    }
    out.push_str("</manifest>\n<spine>\n");
    for ix in 0..sample.chapters.len() {
        out.push_str(&format!("<itemref idref=\"ch{:02}\"/>\n", ix + 1));
    }
    out.push_str("</spine>\n</package>\n");
    out
}

fn nav(sample: &Sample) -> String {
    let mut out = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<html xmlns=\"http://www.w3.org/1999/xhtml\" xmlns:epub=\"http://www.idpf.org/2007/ops\" xml:lang=\"en\"><head><title>{}</title></head>\n<body>\n<nav epub:type=\"toc\"><h1>Contents</h1>\n<ol>\n",
        escape(sample.title)
    );
    for (ix, chapter) in sample.chapters.iter().enumerate() {
        out.push_str(&format!(
            "<li><a href=\"{}\">{}</a></li>\n",
            href(ix),
            escape(chapter.title)
        ));
    }
    out.push_str("</ol>\n</nav>\n</body></html>\n");
    out
}

fn page(chapter: &Chapter) -> String {
    let title = escape(chapter.title);
    let mut out = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<html xmlns=\"http://www.w3.org/1999/xhtml\" xml:lang=\"en\"><head><title>{title}</title></head>\n<body>\n<h1>{title}</h1>\n"
    );
    for paragraph in chapter.paragraphs {
        out.push_str(&format!("<p>{}</p>\n", emphasised(paragraph)));
    }
    out.push_str("</body></html>\n");
    out
}

fn emphasised(text: &str) -> String {
    escape(text)
        .split('_')
        .enumerate()
        .map(|(ix, part)| match ix % 2 {
            0 => part.to_owned(),
            _ => format!("<em>{part}</em>"),
        })
        .collect()
}
