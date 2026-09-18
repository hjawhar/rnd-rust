import numpy as np
import matplotlib.pyplot as plt
from PIL import Image




import os
from flask import Flask 
from flask import request
from paddleocr import PaddleOCR
ocr = PaddleOCR(lang='en', use_gpu=False, show_log=False) 
# img_path = 'https://pbs.twimg.com/media/GhivrlDWAAA7Ex3?format=jpg&name=small'
app = Flask(__name__)


def load_and_print_image(image_path):
    # Load image using PIL and convert to numpy array
    image = Image.open(image_path)
    image_array = np.array(image)
    
    # Print image shape and pixel values
    print(f"Image Shape: {image_array.shape}")
    print("Pixel Data:")
    print(image_array)
    
    # Display the image
    plt.imshow(image_array)
    plt.axis('off')
    plt.show()

# Replace 'your_image.jpg' with the actual image path
# image_path = 'test.jpeg'
# load_and_print_image(image_path)



def delete_files():
    dir_name = "./"
    directory_files = os.listdir(dir_name)
    for item in directory_files:
        if item.endswith(".jpg") or item.endswith(".jpeg") or item.endswith(".png") or item.endswith(".webp") or item.endswith(".gif"):
            os.remove(os.path.join(dir_name, item))

@app.route("/ocr", methods = ['POST'])
def post_ocr():
    try:
            delete_files()
    except:
        print("An exception occurred")
    
    d = dict()
    try:
        data = request.get_json() 
        img_path = data['url']
        result = ocr.ocr(img_path, det=True, cls=False)
    
        for idx in range(len(result)):
            res = result[idx]
            for line in res:
                print(line)

        d['result'] = result
        d['url'] = img_path
        d['ok'] = True
    except:
        d['result'] = []
        d['url'] = img_path
        d['ok'] = False
        return d

    try:
        delete_files()
    except:
        print("An exception occurred")
    return d

if __name__ == "__main__":
    from waitress import serve
    serve(app, host="0.0.0.0", port=6999)