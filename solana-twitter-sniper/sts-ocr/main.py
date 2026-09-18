import requests
import base64
from flask import Flask 
from flask import request
from paddleocr import PaddleOCR
from PIL import Image
import numpy as np
import matplotlib.pyplot as plt
from io import BytesIO
import paddle 
import paddleocr
app = Flask(__name__) 

print("PaddleOCR version" + paddleocr.__version__)  
print("PaddlePaddle version" + paddle.__version__)  

def bytes_to_base64(bytes_data):
    """
    Convert bytes to a base64 encoded string.
    
    Parameters:
    bytes_data (bytes): The bytes data to encode
    
    Returns:
    str: Base64 encoded string
    """
    # Encode the bytes to base64
    base64_encoded = base64.b64encode(bytes_data)
    
    # Convert the base64 bytes to string
    base64_string = base64_encoded.decode('utf-8')
    
    return base64_string

@app.route("/ocr", methods = ['POST'])
def post_ocr():
    data = request.get_json() 
    current_img_path = data['url']
    response = requests.get(current_img_path)

    try: 
        return analyze_img(current_img_path, response)
    except:
        error_dic = dict()
        error_dic['result'] = []
        error_dic['url'] = current_img_path
        error_dic['ok'] = False
        return error_dic
    # print("Status code: "+response.status_code)
    # response.raise_for_status()

    # Functional code
    # base64_result = bytes_to_base64(response.content)
    # print("base64 start")
    # print(base64_result)

    # print("base64 end")

    # for idx in range(len(result)):
    #     res = result[idx]
    #     for line in res:
            # print(line)

    # d['result'] = result
    # d['url'] = current_img_path
    # d['ok'] = True
    # return analyze_img(response)

def analyze_img(img_path, response):
    ocr = PaddleOCR(use_angle_cls=False, lang='en', use_gpu=False, show_log=False, ocr_version="PP-OCRv3")
    image = Image.open(BytesIO(response.content))
    image_array = np.array(image) 
    result = ocr.ocr(image_array, det=True, cls=False)
    d = dict()
    d['result'] = result
    d['url'] = img_path
    d['ok'] = True
    return d

if __name__ == "__main__":
    from waitress import serve
    serve(app, host="0.0.0.0", port=6999)
 
    # for i in range(5): 
    #     current_img_path = "https://pbs.twimg.com/media/Glir16aWwAADlSj?format=jpg&name=small"
    #     response = requests.get(current_img_path)
    #     print("Fetching image "+ str(i+1))
    #     print(analyze_img(response))