// // // use paddle::{det::Det, rec::Rec};

// // // pub mod paddle;

// // use ort::session::{builder::GraphOptimizationLevel, Session};

// // fn main() -> Result<(), Box<dyn std::error::Error>> {
// //     // let model = Session::builder()?
// //     //     .with_optimization_level(GraphOptimizationLevel::Level3)?
// //     //     .with_intra_threads(4)?
// //     //     .commit_from_file("yolov8m.onnx")?;

// //     // let det = Det::from_file("./models/ch_PP-OCRv4_det_infer.onnx")?;
// //     // let rec = Rec::from_file(
// //     //     "./models/ch_PP-OCRv4_rec_infer.onnx",
// //     //     "./models/ppocr_keys_v1.txt",
// //     // )?;
// //     // let img = image::open("./test/test.png")?;
// //     // for sub in det.find_text_img(&img)? {
// //     //     println!("{}", rec.predict_str(&sub)?)
// //     // }

// //     Ok(())
// // }

// use image::{DynamicImage, GenericImageView, Luma};
// use ndarray::Array4;
// use ort::{
//     environment::Environment,
//     session::{builder::GraphOptimizationLevel, Session},
//     value::Value,
// };
// // use ort::{Environment, Session, Value};
// use std::error::Error;

// fn preprocess_image(image: &DynamicImage) -> Array4<f32> {
//     let gray = image.to_luma8();
//     let (width, height) = gray.dimensions();
//     let mut tensor = Array4::<f32>::zeros((1, 1, height as usize, width as usize));

//     for y in 0..height {
//         for x in 0..width {
//             tensor[[0, 0, y as usize, x as usize]] = gray.get_pixel(x, y)[0] as f32 / 255.0;
//         }
//     }

//     tensor
// }

// // fn recognize_text(image_path: &str, model_path: &str) -> Result<(), Box<dyn Error>> {
// //     let environment = Environment::builder().build()?;
// //     let session = Session::builder(&environment)?.with_model_from_file(model_path)?;

// //     Ok(())
// // }

// fn main() -> Result<(), Box<dyn std::error::Error>> {
//     let image_path = "test_image.png";
//     let model_path = "ocr_model.onnx";

//     let model = Session::builder()?
//         .with_optimization_level(GraphOptimizationLevel::Level3)?
//         .with_intra_threads(4)?
//         .commit_from_file("yolov8m.onnx")?;

//         let image = image::open(image_path)?;
//         let input_tensor = preprocess_image(&image);


//         // model.run(input_values)
//     //     let input_value = Value::from_array(session.allocator(), &input_tensor)?;

//     //     let outputs = session.run(vec![input_value])?;

//     //     println!("Raw output: {:?}", outputs[0]); // Post-process this for final text

//     // if let Err(e) = recognize_text(image_path, model_path) {
//     //     eprintln!("Error: {}", e);
//     // }
//     Ok(())
// }


use ort::{Environment, Session, Value, ndarray::Array4};
use image::{DynamicImage, GenericImageView, imageops::resize, imageops::FilterType};
use anyhow::Result;

fn main() -> Result<()> {
    // Initialize ONNX Runtime
    let environment = Environment::builder()
        .with_name("ocr_detector")
        .build()?;

    // Load the OCR detection model
    let model_path = "ch_PP-OCRv4_det_infer.onnx"; // Update with actual path
    let session = Session::builder(&environment)?
        .with_model_from_file(model_path)?;

    // Load and preprocess the image
    let image = image::open("test_image.png")?;
    let input_tensor = preprocess_image(&image)?;

    // Run inference
    let outputs = session.run(ort::inputs!["x" => input_tensor]?)?;
    let detection_result: &Array4<f32> = outputs["out"].extract_tensor()?;

    // Post-process results
    println!("Detection results: {:?}", detection_result.shape());

    Ok(())
}

fn preprocess_image(img: &DynamicImage) -> Result<Value> {
    let resized = resize(img, 640, 640, FilterType::Lanczos3);
    let rgb = resized.to_rgb8();
    let (width, height) = rgb.dimensions();

    // Convert image to a normalized float tensor
    let data: Vec<f32> = rgb
        .pixels()
        .flat_map(|p| [p.0[0] as f32 / 255.0, p.0[1] as f32 / 255.0, p.0[2] as f32 / 255.0])
        .collect();

    let input_tensor = Array4::from_shape_vec((1, 3, height as usize, width as usize), data)?;

    Ok(Value::from_array(input_tensor)?)
}
